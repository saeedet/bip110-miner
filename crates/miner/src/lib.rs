//! A Stratum V1 mining client for the BLAKE2b fork.
//!
//! Connects to a pool, subscribes, and hashes whatever it is sent. It knows
//! nothing about blocks, transactions, or the node — and on this chain it knows
//! remarkably little even about the header it is helping to solve. It never
//! sees a version, a timestamp, a merkle root, or the real previous block hash.
//! All of those are folded into one digest before the job is built, which the
//! node's source says is the point:
//!
//! > *"These fields are invisible to the mining machine. This means the hasher
//! > cannot brick itself at some future block version, time, or difficulty."*
//!
//! That ignorance is what makes this program replaceable. Everything it does,
//! purpose-built hardware would also do, over the same protocol on the same
//! port.
//!
//! # A library, so a dashboard can drive it
//!
//! [`run`] is the whole miner. It reports through a [`Sink`] rather than the
//! terminal, never ends the process, and takes a [`Controls`] handle that can
//! stop it, pause it, or change how many cores it uses while it runs.

mod connection;
mod controls;
mod lifetime;
mod stats;
mod work;
mod worker;

pub use controls::Controls;
pub use lifetime::default_path as default_lifetime_path;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use bip110_primitives::{Target, hex};
use events::{Event, Level, LifetimeReport, Report, Sink};
use serde_json::{Value, json};
use stratum::{Incoming, Job, Request, method};

use lifetime::Lifetime;
use stats::Stats;
use work::{Work, WorkState};

/// Errors from [`run`]. `Send + Sync` so a miner on its own thread can hand its
/// failure back to whoever started it.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// How long to wait for the pool to answer the handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// How often the lifetime totals are written to disk, however often reports go
/// out. A dashboard wants a report every second; the file does not.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);

/// How the miner should run.
#[derive(Debug, Clone)]
pub struct Options {
    /// The pool's address, `host:port`.
    pub pool: String,
    /// The worker name sent to the pool.
    pub worker: String,
    /// How many hashing threads to start. [`Controls::threads`] decides how
    /// many of them hash at any moment; this is the most there can be.
    pub slots: usize,
    /// How often to emit [`Event::Report`].
    pub report_interval: Duration,
    /// Where lifetime totals are kept.
    pub lifetime_path: PathBuf,
}

/// Runs the miner until `controls` asks it to stop or the pool goes away.
///
/// Blocks the calling thread.
pub fn run(options: &Options, sink: Arc<dyn Sink>, controls: Arc<Controls>) -> Result<(), Error> {
    sink.emit(Event::Connecting { pool: options.pool.clone() });
    let connection = connection::connect(&options.pool, Arc::clone(&sink))?;

    // --- mining.subscribe ---------------------------------------------------
    //
    // The reply carries our extranonce1 and the width of the extranonce2 we are
    // expected to supply. Together they fill the header's 16-byte extranonce,
    // and without both, every hash we computed would be against a midstate the
    // pool cannot reproduce.
    connection.outbound.send(serde_json::to_string(&Request::call(
        1,
        method::SUBSCRIBE,
        json!(["bip110-miner/0.1.0"]),
    ))?)?;

    let subscribe_reply = wait_for_response(&connection, 1)?;
    let (extranonce1, extranonce2_size) = parse_subscription(&subscribe_reply)?;

    sink.emit(Event::Subscribed { extranonce1: hex::encode(&extranonce1), extranonce2_size });

    // --- mining.authorize ---------------------------------------------------
    connection.outbound.send(serde_json::to_string(&Request::call(
        2,
        method::AUTHORIZE,
        json!([options.worker, "x"]),
    ))?)?;

    let authorized = wait_for_response(&connection, 2)?;
    if authorized != json!(true) {
        return Err(format!("the pool refused to authorize {:?}", options.worker).into());
    }
    sink.emit(Event::Authorized { worker: options.worker.clone() });

    // --- Run ----------------------------------------------------------------
    let state = Arc::new(WorkState::new());
    let stats = Arc::new(Stats::new());

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    sink.emit(Event::Hashing { threads: controls.threads(), cores });

    let workers = worker::spawn(
        Arc::clone(&state),
        Arc::clone(&stats),
        connection.outbound.clone(),
        options.worker.clone(),
        options.slots.max(controls.threads()),
        Arc::clone(&controls),
        Arc::clone(&sink),
    );

    let reporter = {
        let stats = Arc::clone(&stats);
        let state = Arc::clone(&state);
        let sink = Arc::clone(&sink);
        let controls = Arc::clone(&controls);
        let options = options.clone();
        std::thread::spawn(move || report(&options, &stats, &state, sink.as_ref(), &controls))
    };

    // This thread becomes the network loop: everything the pool sends from
    // here on is either new work or a verdict on a share. It wakes regularly to
    // notice a stop, rather than blocking on the pool indefinitely.
    loop {
        if controls.stopping() {
            break;
        }

        match connection.incoming.recv_timeout(Duration::from_millis(200)) {
            Ok(Incoming::Request(request)) if request.method == method::NOTIFY => {
                match Job::from_notify_params(&request.params) {
                    Ok(job) => install(&state, job, &extranonce1, extranonce2_size, sink.as_ref()),
                    Err(error) => warn(sink.as_ref(), format!("bad job from pool: {error}")),
                }
            }
            Ok(Incoming::Request(request)) if request.method == method::SET_DIFFICULTY => {
                if let Some(difficulty) = request.params.get(0).and_then(Value::as_f64) {
                    sink.emit(Event::PoolDifficulty { difficulty });
                }
            }
            Ok(Incoming::Request(_)) => {}
            Ok(Incoming::Response(response)) => match response.error {
                // The pool checks every share itself, from the header rather
                // than from the job, so a rejection here means our two
                // reconstructions disagreed — worth shouting about, because it
                // means one of us has a bug.
                Some(error) => sink.emit(Event::ShareRejected { reason: error.to_string() }),
                None => sink.emit(Event::ShareAccepted),
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                sink.emit(Event::PoolClosed);
                break;
            }
        }
    }

    // Wind down: hashing threads finish their batch, the reporter saves the
    // totals one last time, and the socket closes.
    controls.stop();
    for handle in workers {
        let _ = handle.join();
    }
    let _ = reporter.join();
    connection.close();
    Ok(())
}

/// Emits a [`Report`] every interval, and keeps the lifetime totals up to date.
///
/// Runs on its own thread so the mining threads never spend time on formatting
/// or disk I/O. Saves once more on the way out, so stopping the miner does not
/// lose the last interval's work.
fn report(options: &Options, stats: &Stats, state: &WorkState, sink: &dyn Sink, controls: &Controls) {
    let mut lifetime = Lifetime::load(&options.lifetime_path);
    let mut folded_in = 0u64;
    let mut last_report = Instant::now();
    let mut last_save = Instant::now();

    sink.emit(Event::LifetimeLoaded {
        total_hashes: lifetime.total_hashes,
        sessions: lifetime.sessions,
        best_zero_bits: lifetime.best_zero_bits,
    });

    loop {
        // Sleep in short steps so a stop is noticed promptly.
        std::thread::sleep(Duration::from_millis(100));
        let stopping = controls.stopping();
        if last_report.elapsed() < options.report_interval && !stopping {
            continue;
        }

        let elapsed = last_report.elapsed().as_secs_f64();
        last_report = Instant::now();

        let total = stats.total_hashes();
        let recent = total - folded_in;
        folded_in = total;

        let (best, zero_bits) = stats.best();
        lifetime.record(recent, best, zero_bits);

        if stopping || last_save.elapsed() >= SAVE_INTERVAL {
            last_save = Instant::now();
            if let Err(error) = lifetime.save() {
                warn(sink, format!("warning: cannot save lifetime totals: {error}"));
            }
        }

        if stopping {
            return;
        }

        // The difficulty being mined and the leading-zero threshold that goes
        // with it. Reporting a best-hash figure without the bar it is measured
        // against invites the wrong reading entirely.
        let (difficulty, needed) = state.snapshot().map_or((0.0, 0), |(work, _)| {
            (Target::difficulty(work.job.bits), work.target.leading_zero_bits())
        });

        sink.emit(Event::Report(Report {
            unix_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            // Over the time that actually passed, not the nominal interval.
            recent_rate: if elapsed > 0.0 { recent as f64 / elapsed } else { 0.0 },
            average_rate: stats.average_hashrate(),
            session_hashes: total,
            best_hash: best.to_string(),
            best_zero_bits: zero_bits,
            needed_zero_bits: needed,
            // Restated every report because it is the only honest measure of
            // progress: the odds are linear in total work and carry no memory
            // of how that work was spread over time.
            lifetime: (difficulty > 0.0).then(|| LifetimeReport {
                total_hashes: lifetime.total_hashes,
                best_zero_bits: lifetime.best_zero_bits,
                odds_denominator: lifetime.odds_denominator(difficulty),
            }),
        }));
    }
}

/// Installs a new job, replacing whatever the miner was working on.
fn install(state: &WorkState, job: Job, extranonce1: &[u8], extranonce2_size: usize, sink: &dyn Sink) {
    let Ok(target) = Target::from_compact(job.bits) else {
        warn(sink, format!("job {} has an undecodable target, ignoring", job.job_id));
        return;
    };

    // There is no block hash to name here. The previous block reaches us only
    // as a tagged hash with its first six bytes blanked, so the job is
    // identified by its id and its target and nothing else — which is all a
    // hasher needs, and rather the point.
    sink.emit(Event::JobReceived {
        job_id: job.job_id.clone(),
        difficulty: Target::difficulty(job.bits),
        clean: job.clean_jobs,
        prev_hash: None,
    });

    state.set(Work { job, extranonce1: extranonce1.to_vec(), extranonce2_size, target });
}

/// Reads until the response with `id` arrives, discarding notifications.
///
/// The pool may push `set_difficulty` or even a job before answering the
/// handshake, so anything that is not the reply we are waiting for is skipped
/// rather than treated as an error.
fn wait_for_response(connection: &connection::Connection, id: u64) -> Result<Value, Error> {
    let deadline = std::time::Instant::now() + HANDSHAKE_TIMEOUT;

    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or("the pool did not answer in time")?;

        match connection.incoming.recv_timeout(remaining)? {
            Incoming::Response(response) if response.id == Some(id) => {
                if let Some(error) = response.error {
                    return Err(format!("the pool returned an error: {error}").into());
                }
                return Ok(response.result);
            }
            _ => continue,
        }
    }
}

/// Pulls extranonce1 and extranonce2_size out of the subscribe reply.
///
/// The reply is `[[[notification, id], ...], extranonce1, extranonce2_size]`.
/// Only the last two elements matter to us.
fn parse_subscription(result: &Value) -> Result<(Vec<u8>, usize), Error> {
    let array = result.as_array().ok_or("subscribe reply is not an array")?;
    if array.len() < 3 {
        return Err(format!("subscribe reply has {} elements, expected 3", array.len()).into());
    }

    let extranonce1 = hex::decode(array[1].as_str().ok_or("subscribe reply has a non-string extranonce1")?)?;

    let extranonce2_size =
        array[2].as_u64().ok_or("subscribe reply has a non-numeric extranonce2_size")? as usize;

    // The two halves fill one 16-byte header field, so a pool that assigns a
    // width leaving them short of it has handed us work we cannot do. Caught
    // here rather than on the first job, when it would be one line of noise per
    // notification.
    if extranonce1.len() + extranonce2_size != 16 {
        return Err(format!(
            "the pool assigned {} + {extranonce2_size} extranonce bytes; this chain's header \
             extranonce is 16",
            extranonce1.len()
        )
        .into());
    }

    Ok((extranonce1, extranonce2_size))
}

/// Reports a warning that does not stop the miner.
fn warn(sink: &dyn Sink, text: String) {
    sink.emit(Event::Log { level: Level::Warn, text });
}
