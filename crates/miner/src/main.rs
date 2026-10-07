//! The miner on its own.
//!
//! ```text
//! miner [--pool 127.0.0.1:3334] [--worker mac] [--threads N|half|max]
//! ```
//!
//! Everything the miner does is in the library; this only reads arguments and
//! prints what it reports.

use std::sync::Arc;
use std::time::Duration;

use events::Plain;
use miner::{Controls, Options};

/// How often to print a status block.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);

fn main() {
    let result = parse_args().and_then(|(options, threads)| {
        miner::run(&options, Arc::new(Plain), Arc::new(Controls::new(threads)))
    });

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn parse_args() -> Result<(Options, usize), miner::Error> {
    // Leave two cores free by default.
    //
    // Using every core costs roughly a third more heat and fan noise for the
    // last ~20% of hashrate, and makes the machine unpleasant to use. Since the
    // expected time to a block is measured in geological units either way,
    // trading a fifth of the hashrate for a usable laptop is not a meaningful
    // sacrifice. `--threads max` overrides this.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let default_threads = cores.saturating_sub(2).max(1);

    let mut pool = "127.0.0.1:3334".to_owned();
    let mut worker = "mac".to_owned();
    let mut threads = default_threads;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));

        match flag.as_str() {
            "--pool" => pool = value()?,
            "--worker" => worker = value()?,
            "--threads" => {
                let requested = value()?;
                threads = match requested.as_str() {
                    // On an Apple Silicon chip with an even split of
                    // performance and efficiency cores, "half" lands roughly on
                    // the performance cores alone — about 70% of full hashrate
                    // for appreciably less heat.
                    "half" => (cores / 2).max(1),
                    "max" => cores,
                    number => number.parse()?,
                };
                if threads == 0 {
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

    let options = Options {
        pool,
        worker,
        slots: threads,
        report_interval: REPORT_INTERVAL,
        lifetime_path: miner::default_lifetime_path(),
    };
    Ok((options, threads))
}
