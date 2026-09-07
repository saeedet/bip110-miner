//! A Stratum V1 mining client for the BLAKE2b fork.
//!
//! ```text
//! miner [--pool 127.0.0.1:3334] [--worker mac] [--threads N|half|max]
//! ```
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

mod connection;
mod stats;
mod work;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use btcb2_primitives::{Target, hex};
use serde_json::{Value, json};
use stratum::{Incoming, Job, Request, method};

use stats::Stats;
use work::{Work, WorkState};

/// How long to wait for the pool to answer the handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// How often to print a status line.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_args()?;
    let (pool_address, worker_name) = (options.pool, options.worker);

    println!("connecting to {pool_address}");
    let connection = connection::connect(&pool_address)?;

    // --- mining.subscribe ---------------------------------------------------
    //
    // The reply carries our extranonce1 and the width of the extranonce2 we are
    // expected to supply. Together they fill the header's 16-byte extranonce,
    // and without both, every hash we computed would be against a midstate the
    // pool cannot reproduce.
    connection.outbound.send(serde_json::to_string(&Request::call(
        1,
        method::SUBSCRIBE,
        json!(["btcb2-miner/0.1.0"]),
    ))?)?;

    let subscribe_reply = wait_for_response(&connection, 1)?;
    let (extranonce1, extranonce2_size) = parse_subscription(&subscribe_reply)?;

    println!(
        "subscribed: extranonce1 {}, extranonce2 {} bytes",
        hex::encode(&extranonce1),
        extranonce2_size
    );

    // --- mining.authorize ---------------------------------------------------
    connection.outbound.send(serde_json::to_string(&Request::call(
        2,
        method::AUTHORIZE,
        json!([worker_name, "x"]),
    ))?)?;

    let authorized = wait_for_response(&connection, 2)?;
    if authorized != json!(true) {
        return Err(format!("the pool refused to authorize {worker_name:?}").into());
    }
    println!("authorized as {worker_name}\n");

    // --- Run ----------------------------------------------------------------
    let state = Arc::new(WorkState::new());
    let stats = Arc::new(Stats::new());

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    println!(
        "hashing on {} of {cores} cores{}\n",
        options.threads,
        if options.threads >= cores { " (full tilt — expect heat)" } else { "" },
    );

    worker::spawn(
        Arc::clone(&state),
        Arc::clone(&stats),
        connection.outbound.clone(),
        worker_name,
        options.threads,
    );

    {
        let stats = Arc::clone(&stats);
        let state = Arc::clone(&state);
        std::thread::spawn(move || report_forever(&stats, &state));
    }

    // The main thread becomes the network loop: everything the pool sends from
    // here on is either new work or a verdict on a share.
    for message in connection.incoming {
        match message {
            Incoming::Request(request) if request.method == method::NOTIFY => {
                match Job::from_notify_params(&request.params) {
                    Ok(job) => install(&state, job, &extranonce1, extranonce2_size),
                    Err(error) => eprintln!("bad job from pool: {error}"),
                }
            }
            Incoming::Request(request) if request.method == method::SET_DIFFICULTY => {
                if let Some(difficulty) = request.params.get(0).and_then(Value::as_f64) {
                    println!("pool set difficulty to {difficulty}");
                }
            }
            Incoming::Request(_) => {}
            Incoming::Response(response) => match response.error {
                // The pool checks every share itself, from the header rather
                // than from the job, so a rejection here means our two
                // reconstructions disagreed — worth shouting about, because it
                // means one of us has a bug.
                Some(error) => eprintln!("share rejected: {error}"),
                None => println!("share accepted"),
            },
        }
    }

    println!("the pool closed the connection");
    Ok(())
}

/// Formats a hash count with an SI-style suffix.
fn si(count: u64) -> String {
    const UNITS: [(f64, &str); 5] = [
        (1e18, "E"),
        (1e15, "P"),
        (1e12, "T"),
        (1e9, "G"),
        (1e6, "M"),
    ];
    let value = count as f64;

    for (scale, suffix) in UNITS {
        if value >= scale {
            return format!("{:.2}{suffix}", value / scale);
        }
    }
    format!("{count}")
}

/// Prints a status line every [`REPORT_INTERVAL`].
///
/// Runs on its own thread so the mining threads never spend time on formatting,
/// and so the interval stays honest regardless of batch length.
fn report_forever(stats: &Stats, state: &WorkState) {
    let mut folded_in = 0u64;

    loop {
        std::thread::sleep(REPORT_INTERVAL);

        let total = stats.total_hashes();
        let recent = total - folded_in;
        folded_in = total;

        let (best, zero_bits) = stats.best();

        // The leading-zero threshold the target actually demands. Reporting a
        // best-hash figure without the bar it is measured against invites the
        // wrong reading entirely: 36 of 79 looks like halfway and is in fact
        // 2^43 — some eight trillion times — too easy.
        let needed = state
            .snapshot()
            .map_or(0, |(work, _)| work.target.leading_zero_bits());

        println!(
            "{:>7.2} MH/s (avg {:>6.2})   session {:>8}   best {zero_bits}/{needed} bits   {best}",
            recent as f64 / REPORT_INTERVAL.as_secs_f64() / 1e6,
            stats.average_hashrate() / 1e6,
            si(total),
        );
    }
}

/// Installs a new job, replacing whatever the miner was working on.
fn install(state: &WorkState, job: Job, extranonce1: &[u8], extranonce2_size: usize) {
    let Ok(target) = Target::from_compact(job.bits) else {
        eprintln!("job {} has an undecodable target, ignoring", job.job_id);
        return;
    };

    // There is no block hash to name here. The previous block reaches us only
    // as a tagged hash with its first six bytes blanked, so the job is
    // identified by its id and its target and nothing else — which is all a
    // hasher needs, and rather the point.
    println!(
        "job {} at difficulty {:.4}{}",
        job.job_id,
        Target::difficulty(job.bits),
        if job.clean_jobs { " (clean)" } else { "" }
    );

    state.set(Work {
        job,
        extranonce1: extranonce1.to_vec(),
        extranonce2_size,
        target,
    });
}

/// Reads until the response with `id` arrives, discarding notifications.
///
/// The pool may push `set_difficulty` or even a job before answering the
/// handshake, so anything that is not the reply we are waiting for is skipped
/// rather than treated as an error.
fn wait_for_response(
    connection: &connection::Connection,
    id: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
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
fn parse_subscription(result: &Value) -> Result<(Vec<u8>, usize), Box<dyn std::error::Error>> {
    let array = result.as_array().ok_or("subscribe reply is not an array")?;
    if array.len() < 3 {
        return Err(format!("subscribe reply has {} elements, expected 3", array.len()).into());
    }

    let extranonce1 = hex::decode(
        array[1]
            .as_str()
            .ok_or("subscribe reply has a non-string extranonce1")?,
    )?;

    let extranonce2_size = array[2]
        .as_u64()
        .ok_or("subscribe reply has a non-numeric extranonce2_size")? as usize;

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

/// Command-line options.
struct Options {
    pool: String,
    worker: String,
    threads: usize,
}

fn parse_args() -> Result<Options, Box<dyn std::error::Error>> {
    // Leave two cores free by default.
    //
    // Using every core costs roughly a third more heat and fan noise for the
    // last ~20% of hashrate, and makes the machine unpleasant to use. Since the
    // expected time to a block is measured in geological units either way,
    // trading a fifth of the hashrate for a usable laptop is not a meaningful
    // sacrifice. `--threads max` overrides this.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let default_threads = cores.saturating_sub(2).max(1);

    let mut options = Options {
        pool: "127.0.0.1:3334".to_owned(),
        worker: "mac".to_owned(),
        threads: default_threads,
    };

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));

        match flag.as_str() {
            "--pool" => options.pool = value()?,
            "--worker" => options.worker = value()?,
            "--threads" => {
                let requested = value()?;
                options.threads = match requested.as_str() {
                    // On an Apple Silicon chip with an even split of
                    // performance and efficiency cores, "half" lands roughly on
                    // the performance cores alone — about 70% of full hashrate
                    // for appreciably less heat.
                    "half" => (cores / 2).max(1),
                    "max" => cores,
                    number => number.parse()?,
                };
                if options.threads == 0 {
                    return Err("--threads must be at least 1".into());
                }
            }
            "--help" | "-h" => {
                println!(
                    "miner [--pool ADDR] [--worker NAME] [--threads N|half|max]\n\
                     \n\
                     --threads defaults to {default_threads} of {cores} cores, leaving two free so\n\
                     the machine stays usable. `half` is {} and `max` is {cores}.",
                    (cores / 2).max(1),
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other:?}").into()),
        }
    }

    Ok(options)
}
