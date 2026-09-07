//! The hashing loop, run on every core at once.
//!
//! # How the work is divided
//!
//! Threads do not split a nonce range. Each claims its **own extranonce2** from
//! a shared counter, which gives it a different stage-3 midstate and therefore
//! a disjoint search space. That costs one atomic increment per thread per
//! *session* — which is to say, no coordination at all — and it scales to any
//! number of threads without them ever having to agree on anything.
//!
//! The sibling Bitcoin project does the same thing, but has to reclaim a fresh
//! extranonce every 2³² hashes because that is all the space one gives it. Here
//! a single extranonce2 covers 2⁶⁴ hashes — the 32-bit nonce crossed with the
//! 32-bit time offset — so a thread claims one and never comes back.
//!
//! # The traversal, and why this module owns it
//!
//! Two of the header's four nonce words are unreachable through Stratum V1:
//! `mining.submit` has five parameters and no slot for `nonce2` or `nonce3`, so
//! a share carrying them could not be described to the pool, and the pool
//! forces them to zero when it re-checks. This loop must therefore roll `nonce`
//! and `time_offset` *only*.
//!
//! [`mining::search`] carries between all four words, which is right when
//! mining a header directly and wrong here. So the traversal lives in this
//! module: batches are sized and aligned so a sweep never reaches the carry,
//! and the outer loop advances `time_offset` itself.
//!
//! # Batching
//!
//! Nonces are tried in batches so that between batches a thread can notice new
//! work. Too small and the checks cost real time; too large and the miner keeps
//! grinding a job the chain has moved past.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use mining::NonceSpace;
use stratum::{Request, Share, method};

use crate::stats::Stats;
use crate::work::WorkState;

/// Nonces per batch, between checks for new work.
///
/// A power of two that divides 2³² exactly, so batches tile the nonce word and
/// none of them ever straddles the wrap. That alignment is what keeps the carry
/// out of `nonce2` — see the module docs.
const BATCH: u32 = 1 << 20;

/// How long to wait for fresh work after finding a block candidate.
///
/// Bounded so that a candidate which loses its race — or a pool that goes quiet
/// — cannot stall a thread indefinitely.
const NEW_WORK_TIMEOUT: Duration = Duration::from_secs(2);

/// Everything the mining threads share.
struct Shared {
    state: Arc<WorkState>,
    stats: Arc<Stats>,
    outbound: Sender<String>,
    worker: String,
    /// Handed out one per thread, so no two threads share a midstate.
    extranonce_counter: AtomicU64,
    /// JSON-RPC ids for submissions.
    submit_id: AtomicU64,
}

/// Starts `threads` mining threads and returns immediately.
pub fn spawn(
    state: Arc<WorkState>,
    stats: Arc<Stats>,
    outbound: Sender<String>,
    worker: String,
    threads: usize,
) {
    let shared = Arc::new(Shared {
        state,
        stats,
        outbound,
        worker,
        extranonce_counter: AtomicU64::new(0),
        submit_id: AtomicU64::new(100),
    });

    for _ in 0..threads {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || mine(&shared));
    }
}

/// One mining thread, running until the connection drops.
fn mine(shared: &Shared) {
    // Claimed once, for the life of the thread. 2^64 hashes sit under it.
    let ticket = shared.extranonce_counter.fetch_add(1, Ordering::Relaxed);

    loop {
        let Some((work, generation)) = shared.state.snapshot() else {
            // No job yet. Wait rather than spin.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };

        // Another thread already solved this job. Wait for the next one rather
        // than looping straight back here and burning a core on nothing.
        if !shared.state.is_current(generation) {
            wait_for_new_work(&shared.state, generation);
            continue;
        }

        let extranonce2 = encode_extranonce2(ticket, work.extranonce2_size);

        // Stages 0 to 3, computed once and reused for everything below. The
        // pool did stages 0 to 2 and sent the results; this is the one BLAKE2b
        // that turns them into something a nonce can be tried against.
        let midstate = match work.job.midstate(&work.extranonce1, &extranonce2) {
            Ok(midstate) => midstate,
            Err(error) => {
                eprintln!("cannot build a midstate for this job: {error}");
                wait_for_new_work(&shared.state, generation);
                continue;
            }
        };

        let mut at = NonceSpace::default();

        while shared.state.is_current(generation) {
            let result = mining::search(&midstate, &work.target, at, u64::from(BATCH));

            shared.stats.record(result.hashes, result.best);

            if let Some(solution) = result.solution {
                // Stop every thread on this job, not just this one.
                shared.state.mark_solved(generation);
                submit(shared, &work.job.job_id, &extranonce2, solution);

                // The pool announces the network difficulty, so a solution is a
                // block candidate rather than a partial share. However it turns
                // out, the tip is about to move — so there is nothing left worth
                // doing with this job.
                wait_for_new_work(&shared.state, generation);
                break;
            }

            at = next_batch(at);
        }
    }
}

/// Advances to the next batch, rolling `nonce` then `time_offset`.
///
/// `nonce2` and `nonce3` are deliberately never touched: Stratum has no way to
/// report them, so a hash found with either set could not be submitted. This is
/// the whole reason the traversal is here and not in [`mining::search`], which
/// carries across all four words.
fn next_batch(at: NonceSpace) -> NonceSpace {
    match at.nonce.checked_add(BATCH) {
        Some(nonce) => NonceSpace { nonce, ..at },
        // The nonce word is exhausted. Roll the offset and start it again.
        None => NonceSpace {
            nonce: 0,
            time_offset: at.time_offset.wrapping_add(1),
            ..at
        },
    }
}

/// Sends a share to the pool.
fn submit(shared: &Shared, job_id: &str, extranonce2: &[u8], solution: mining::Solution) {
    println!(
        "solution found: {} ({} zero bits)",
        solution.hash,
        solution.hash.leading_zero_bits()
    );

    // Stratum cannot carry these, and the pool zeroes them when it re-checks,
    // so a non-zero value here would produce a share the pool computes a
    // different hash for — rejected, with nothing to point at. The traversal
    // above cannot produce one; this catches the day somebody changes it.
    if solution.at.nonce2 != 0 || solution.at.nonce3 != 0 {
        eprintln!(
            "refusing to submit a share with nonce2={} nonce3={}: Stratum has no slot \
             for them, so the pool would reconstruct a different hash",
            solution.at.nonce2, solution.at.nonce3
        );
        return;
    }

    let share = Share {
        worker: shared.worker.clone(),
        job_id: job_id.to_owned(),
        extranonce2: extranonce2.to_vec(),
        time_offset: solution.at.time_offset,
        nonce: solution.at.nonce,
    };

    let id = shared.submit_id.fetch_add(1, Ordering::Relaxed);

    match serde_json::to_string(&Request::call(id, method::SUBMIT, share.to_submit_params())) {
        Ok(line) => {
            let _ = shared.outbound.send(line);
        }
        Err(error) => eprintln!("cannot serialise share: {error}"),
    }
}

/// Blocks until *new work arrives*, or the timeout expires.
///
/// Watches the generation rather than `is_current`, because `is_current` also
/// goes false when this generation is solved — which is exactly the moment this
/// function is called, so using it here would return immediately every time.
fn wait_for_new_work(state: &WorkState, generation: u64) {
    let deadline = Instant::now() + NEW_WORK_TIMEOUT;

    while Instant::now() < deadline {
        if state.generation() != generation {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Encodes the extranonce ticket into exactly `size` bytes, big-endian.
///
/// Truncates from the top when the counter outgrows the width. With one ticket
/// per thread that needs more threads than a machine has, so it is a guard
/// rather than a case.
fn encode_extranonce2(counter: u64, size: usize) -> Vec<u8> {
    let bytes = counter.to_be_bytes();
    let mut extranonce2 = vec![0u8; size];

    let copy = size.min(bytes.len());
    extranonce2[size - copy..].copy_from_slice(&bytes[bytes.len() - copy..]);

    extranonce2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The batches must tile the nonce word exactly, with none of them
    /// straddling the wrap.
    ///
    /// This is what keeps a carry from reaching `nonce2`. If `BATCH` stopped
    /// dividing 2³², a sweep would eventually run past `u32::MAX` inside a
    /// single `search` call, and every hash after that point would carry a
    /// `nonce2` the pool forces back to zero — shares that reconstruct to a
    /// different hash and are rejected with nothing to point at.
    #[test]
    fn batches_tile_the_nonce_word() {
        assert_eq!((1u64 << 32) % u64::from(BATCH), 0, "BATCH must divide 2^32");

        let mut at = NonceSpace::default();
        let mut covered = 0u64;

        loop {
            covered += u64::from(BATCH);
            at = next_batch(at);
            if at.nonce == 0 {
                break;
            }
        }

        assert_eq!(covered, 1u64 << 32, "every nonce, exactly once");
        assert_eq!(at.time_offset, 1, "and the offset advanced once");
        assert_eq!(at.nonce2, 0, "without disturbing nonce2");
        assert_eq!(at.nonce3, 0, "or nonce3");
    }

    /// An ordinary step moves only the nonce.
    #[test]
    fn a_batch_step_touches_nothing_else() {
        let at = next_batch(NonceSpace { nonce: 0, nonce2: 0, nonce3: 0, time_offset: 5 });
        assert_eq!(at, NonceSpace { nonce: BATCH, nonce2: 0, nonce3: 0, time_offset: 5 });
    }

    #[test]
    fn extranonce2_fills_the_assigned_width() {
        assert_eq!(encode_extranonce2(1, 8), vec![0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(encode_extranonce2(0x0102, 8), vec![0, 0, 0, 0, 0, 0, 1, 2]);
        // A counter wider than the field keeps its low bytes.
        assert_eq!(encode_extranonce2(0x0102_0304_0506, 2), vec![5, 6]);
    }

    /// Every thread must get a distinct extranonce, or two of them build the
    /// same midstate and half the machine's work is wasted.
    #[test]
    fn claimed_extranonces_are_distinct() {
        let counter = AtomicU64::new(0);
        let claimed: Vec<_> = (0..64)
            .map(|_| encode_extranonce2(counter.fetch_add(1, Ordering::Relaxed), 8))
            .collect();

        let unique: std::collections::HashSet<_> = claimed.iter().collect();
        assert_eq!(unique.len(), claimed.len());
    }
}
