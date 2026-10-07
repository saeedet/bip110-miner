//! `bip110-miner` / `bip110-miner start` — mine.
//!
//! Brings up whatever is missing — the node, then the pool, then the miner —
//! in one process, and stops the pool and miner together on Ctrl-C. The pool
//! still listens on its usual port, so other mining hardware can join in.
//!
//! The node is left running afterwards; see [`crate::node`].

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use events::{Event, Plain, Sink};
use miner::Controls;
use node_rpc::Network;

use crate::config::{Settings, network_key};
use crate::node;
use crate::platform::KeepAwake;

/// How often the miner reports its hashrate in plain output.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// Runs the command.
pub fn run(settings: &Settings, keep_awake: bool) -> Result<(), String> {
    // Real money needs a deliberate address. Regtest makes its own.
    if settings.address.is_none() && settings.network != Network::Regtest {
        return Err(format!(
            "no reward address for {} yet. Save one with:\n\n    bip110-miner setup --address <your address>",
            network_key(settings.network)
        ));
    }

    ensure_node(settings)?;

    let sink = Arc::new(Watch::default());
    let stop_pool = Arc::new(AtomicBool::new(false));
    let controls = Arc::new(Controls::new(settings.threads));

    // Ctrl-C (or a SIGTERM) asks both halves to finish, which lets the miner
    // save its lifetime totals. A second press means "now".
    {
        let stop_pool = Arc::clone(&stop_pool);
        let controls = Arc::clone(&controls);
        let presses = AtomicUsize::new(0);
        ctrlc::set_handler(move || {
            if presses.fetch_add(1, Ordering::Relaxed) > 0 {
                std::process::exit(130);
            }
            println!("\nstopping...");
            stop_pool.store(true, Ordering::Relaxed);
            controls.stop();
        })
        .map_err(|error| format!("cannot handle Ctrl-C: {error}"))?;
    }

    let pool = {
        let options = pool::Options {
            network: settings.network,
            listen: settings.listen,
            address: settings.address.clone(),
            datadir: settings.datadir.clone(),
            rpc_port: Some(settings.rpc_port),
        };
        let sink: Arc<dyn Sink> = sink.clone();
        let stop = Arc::clone(&stop_pool);
        std::thread::spawn(move || pool::run(&options, sink, stop))
    };

    // The miner needs real work to do, not merely an open port.
    if !sink.wait_until_ready(|| pool.is_finished() || stop_pool.load(Ordering::Relaxed)) {
        controls.stop();
        return finish(pool.join(), Ok(()));
    }

    // A laptop that idles to sleep stops mining without a word.
    let awake = keep_awake.then(KeepAwake::start);
    if awake.as_ref().is_some_and(KeepAwake::active) {
        println!("keeping this machine awake while mining (--allow-sleep to turn this off)\n");
    }

    let miner = {
        let options = miner::Options {
            pool: settings.listen.to_string(),
            worker: worker_name(),
            slots: settings.threads,
            report_interval: REPORT_INTERVAL,
            lifetime_path: lifetime_path(settings.network),
        };
        let sink: Arc<dyn Sink> = sink.clone();
        let controls = Arc::clone(&controls);
        std::thread::spawn(move || miner::run(&options, sink, controls))
    };

    // Whichever half ends first ends the other: a miner with no pool has
    // nothing to hash, and a pool with no miner is not what was asked for.
    while !pool.is_finished() && !miner.is_finished() {
        std::thread::sleep(Duration::from_millis(200));
    }
    stop_pool.store(true, Ordering::Relaxed);
    controls.stop();

    let miner_result = miner.join().unwrap_or_else(|_| Err("the miner panicked".into()));
    finish(pool.join(), miner_result)
}

/// Starts the node if needed, and refuses one too old to mine with.
fn ensure_node(settings: &Settings) -> Result<(), String> {
    if node::is_running(settings) {
        let client = node::client(settings)?;
        let height = client.get_blockchain_info().map(|info| info.blocks).unwrap_or(0);
        println!("node already running at height {height}");
    } else {
        println!("starting the {} node...", network_key(settings.network));
        node::start(settings, |_| {})?;
        let height = node::client(settings)?.get_blockchain_info().map(|info| info.blocks).unwrap_or(0);
        println!("node up at height {height}");
    }

    // The running node is what builds the templates, so it is the one that
    // must be new enough — not whatever happens to be installed.
    let client = node::client(settings)?;
    if let Some(version) = node::running_version(&client)
        && version < node::MIN_KNOTS
    {
        return Err(format!(
            "the running node is Bitcoin Knots {}, which predates the rule change at block \
             973,440. Blocks built on its templates could be rejected. Update to {} or newer, \
             then restart it with: bip110-miner stop",
            node::version_string(version),
            node::version_string(node::MIN_KNOTS),
        ));
    }
    Ok(())
}

/// Turns the two halves' results into the command's.
fn finish(
    pool: std::thread::Result<Result<(), pool::Error>>,
    miner: Result<(), miner::Error>,
) -> Result<(), String> {
    match pool {
        Err(_) => return Err("the pool panicked".into()),
        // Already reported through the sink, in the pool's own words.
        Ok(Err(error)) if error.downcast_ref::<pool::Reported>().is_some() => {
            return Err(String::new());
        }
        Ok(Err(error)) => return Err(error.to_string()),
        Ok(Ok(())) => {}
    }
    miner.map_err(|error| error.to_string())
}

/// Where lifetime totals are kept: one file per network.
///
/// A regtest session finds hundreds of blocks a minute and a mainnet one may
/// never find any, so mixing their hash counts would make the mainnet odds
/// line meaningless. Mainnet keeps the original file name, so existing totals
/// stay where they were.
fn lifetime_path(network: Network) -> std::path::PathBuf {
    let dir = crate::platform::state_dir();
    match network {
        Network::Mainnet => dir.join("lifetime.json"),
        other => dir.join(format!("lifetime.{}.json", network_key(other))),
    }
}

/// The name this machine gives itself to the pool.
fn worker_name() -> String {
    let host = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "mac".to_owned());
    format!("{host}.0")
}

/// Prints events as plain lines, and notices when the pool has work.
#[derive(Default)]
struct Watch {
    ready: Mutex<bool>,
    changed: Condvar,
}

impl Watch {
    /// Waits for the pool's first job. False if `give_up` says to stop first.
    fn wait_until_ready(&self, give_up: impl Fn() -> bool) -> bool {
        let mut ready = self.ready.lock().expect("watch lock");
        while !*ready {
            if give_up() {
                return false;
            }
            ready = self
                .changed
                .wait_timeout(ready, Duration::from_millis(200))
                .expect("watch lock")
                .0;
        }
        true
    }
}

impl Sink for Watch {
    fn emit(&self, event: Event) {
        if event == Event::PoolReady {
            *self.ready.lock().expect("watch lock") = true;
            self.changed.notify_all();
        }
        Plain.emit(event);
    }
}
