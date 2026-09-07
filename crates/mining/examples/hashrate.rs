//! Measures the mining loop's throughput.
//!
//! ```text
//! cargo run --release --example hashrate -p mining
//! ```
//!
//! Reports single-threaded and multi-threaded rates, and converts them into the
//! only figure that matters: expected time to a block at a given difficulty.
//!
//! # Why this is a fair measurement
//!
//! It runs the real [`search`] against a real [`PowMidstate`], including the
//! best-hash tracking and target comparison a live miner pays for. The target
//! is set unreachably low so the loop never exits early.

use std::time::Instant;

use btcb2_primitives::{BlockHeader, Hash256, PowMidstate, Target};
use mining::{NonceSpace, search};

/// Hashes per thread per measurement.
const SAMPLE: u64 = 1 << 21;

fn main() {
    let header = BlockHeader {
        header_v2: true,
        version: 0x2000_0000,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::ZERO,
        time_on_wire: 1_788_787_444,
        bits: 0x1d00_ffff,
        nonce: 0,
        nonce2: 0,
        nonce3: 0,
        extranonce: [0u8; 16],
        time_offset: 0,
        txcount: 1,
        flags: 0,
        xor_key_mask_clear_bits: 0,
        xor_key: [0u8; 16],
        height: 150_615,
        mm_rhs: Hash256::ZERO,
    };

    // A target nothing will meet, so the loop always runs to its full budget.
    let target = Target::from_compact(0x0300_0001).expect("valid");
    let midstate = PowMidstate::new(&header);

    println!("BLAKE2b proof of work — stages 4 and 5, the part a miner repeats\n");

    let single = measure(1, &midstate, &target);
    println!("{:>3} thread   {:>8.2} MH/s", 1, single / 1e6);

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let mut best = single;

    for threads in [2, cores / 2, cores].into_iter().filter(|n| *n > 1) {
        let rate = measure(threads, &midstate, &target);
        best = best.max(rate);
        println!(
            "{threads:>3} threads  {:>8.2} MH/s   ({:.2}x)",
            rate / 1e6,
            rate / single
        );
    }

    println!("\nexpected time to a block, at {:.2} MH/s:", best / 1e6);
    for (label, difficulty) in [
        ("testnet4 minimum (difficulty 1)", 1.0),
        ("testnet4 real difficulty", 1.66e9),
        ("BTCB2 mainnet, early September 2026", 3.5e6),
    ] {
        // p = 1 / (difficulty * 2^32), so the expected number of hashes is the
        // reciprocal. Mining is memoryless: this is a mean, not a countdown,
        // and no amount of prior work brings it closer.
        let seconds = difficulty * 4_294_967_296.0 / best;
        println!("  {label:<38} {}", human(seconds));
    }
}

/// Runs `threads` search loops for a fixed budget and returns hashes/second.
fn measure(threads: usize, midstate: &PowMidstate, target: &Target) -> f64 {
    let started = Instant::now();

    std::thread::scope(|scope| {
        for thread in 0..threads {
            scope.spawn(move || {
                // A distinct starting point per thread, as the real miner does.
                let start = NonceSpace {
                    nonce2: thread as u32,
                    ..Default::default()
                };
                search(midstate, target, start, SAMPLE);
            });
        }
    });

    (SAMPLE * threads as u64) as f64 / started.elapsed().as_secs_f64()
}

/// Formats a duration in whatever unit makes it legible.
fn human(seconds: f64) -> String {
    const UNITS: [(f64, &str); 5] = [
        (31_557_600.0, "years"),
        (86_400.0, "days"),
        (3_600.0, "hours"),
        (60.0, "minutes"),
        (1.0, "seconds"),
    ];

    for (scale, name) in UNITS {
        if seconds >= scale {
            return format!("{:.1} {name}", seconds / scale);
        }
    }
    format!("{seconds:.2} seconds")
}
