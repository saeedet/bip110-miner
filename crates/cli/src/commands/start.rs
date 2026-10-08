//! `bip110-miner` / `bip110-miner start` — mine.
//!
//! Brings up whatever is missing — the node, then the pool, then the miner —
//! in one process, and stops the pool and miner together on `q` or Ctrl-C.
//! The pool listens on this computer only (127.0.0.1), so nothing on the
//! network can reach it.
//!
//! In a terminal this shows the live dashboard. Anywhere else — a log file, a
//! service, a pipe — or with `--plain`, it prints the same events as lines.
//!
//! The node is left running afterwards; see [`crate::node`].

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use events::{Channel, Event, Plain, Sink};
use miner::Controls;
use node_rpc::Network;

use crate::config::{Settings, network_key};
use crate::dashboard;
use crate::node;
use crate::platform::KeepAwake;

/// How often the miner reports its hashrate in plain output.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// How often it reports to the dashboard, whose hashrate graph wants detail.
const DASHBOARD_REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// How to mine.
pub struct Options {
    /// Keep the machine from sleeping while mining.
    pub keep_awake: bool,
    /// Print lines even in a terminal, instead of the dashboard.
    pub plain: bool,
}

/// Runs the command.
pub fn run(settings: &Settings, options: &Options) -> Result<(), String> {
    let live = !options.plain && std::io::stdout().is_terminal();

    // In a terminal, anything missing is asked about right here.
    let mut settings = settings.clone();
    if live {
        match crate::setup::run(&mut settings, false)? {
            crate::setup::Outcome::Ready => {}
            crate::setup::Outcome::Quit => return Ok(()),
        }
    }
    // The mining port, moved along if something already holds it.
    let mut notes = Vec::new();
    if std::net::TcpListener::bind(settings.listen).is_err() {
        let busy = settings.listen.port();
        let free = crate::platform::free_port_from(busy + 1)
            .ok_or_else(|| format!("port {busy} and the next few are all in use"))?;
        settings.listen.set_port(free);
        notes.push(format!(
            "port {busy} is in use — perhaps another copy of {} — so miners connect on {free} this time",
            crate::chain::PROGRAM
        ));
    }
    let settings = &settings;

    // Real money needs a deliberate address. Regtest makes its own.
    if settings.address.is_none() && settings.network != Network::Regtest {
        return Err(format!(
            "no reward address for {} yet. Save one with:\n\n    bip110-miner setup --address <your address>",
            network_key(settings.network)
        ));
    }

    ensure_node(settings)?;
    if settings.network == Network::Regtest
        && let Some(note) = crate::chain::prepare_regtest(&node::client(settings)?)?
    {
        notes.push(note);
    }

    let (sender, events) = channel();
    let output: Arc<dyn Sink> = if live { Arc::new(Channel::new(sender)) } else { Arc::new(Plain) };
    let sink = Arc::new(Watch::new(output));
    let stop_pool = Arc::new(AtomicBool::new(false));
    let controls = Arc::new(Controls::new(settings.threads));
    handle_signals(&stop_pool, &controls, !live)?;

    // A laptop that idles to sleep stops mining without a word.
    if !live {
        for note in &notes {
            println!("{note}");
        }
    }
    let awake = options.keep_awake.then(KeepAwake::start);
    let keeping_awake = awake.as_ref().is_some_and(KeepAwake::active);
    if keeping_awake && !live {
        println!("keeping this machine awake while mining (--allow-sleep to turn this off)\n");
    }

    let session = {
        let settings = settings.clone();
        let (sink, stop_pool, controls) = (Arc::clone(&sink), Arc::clone(&stop_pool), Arc::clone(&controls));
        let interval = if live { DASHBOARD_REPORT_INTERVAL } else { REPORT_INTERVAL };
        std::thread::spawn(move || mine(&settings, &sink, &stop_pool, &controls, interval))
    };

    let shown = live.then(|| {
        dashboard::run(dashboard::Session {
            settings,
            events,
            controls: Arc::clone(&controls),
            finished: &|| session.is_finished(),
            keeping_awake,
            notes,
        })
    });

    // However the screen closed, mining ends with it. Plain output has no
    // screen to close: it runs until a signal or a failure ends the session.
    if live {
        if !session.is_finished() {
            println!("stopping...");
        }
        stop_pool.store(true, Ordering::Relaxed);
        controls.stop();
    }
    let result = session.join().unwrap_or_else(|_| Err("mining stopped unexpectedly".into()));
    drop(awake);

    if let Some(shown) = shown {
        let outcome = shown?;
        // The dashboard has gone, so whatever ended mining is said again here,
        // where it stays readable.
        if let Some(fatal) = &outcome.state.fatal {
            eprintln!("error: {fatal}");
        }
        summarise(&outcome.state);
    }
    result
}

/// Pool, then miner, until either stops or a stop is asked for.
fn mine(
    settings: &Settings,
    sink: &Arc<Watch>,
    stop_pool: &Arc<AtomicBool>,
    controls: &Arc<Controls>,
    report_interval: Duration,
) -> Result<(), String> {
    let pool = {
        let options = pool::Options {
            network: settings.network,
            listen: settings.listen,
            address: settings.address.clone(),
            datadir: settings.datadir.clone(),
            rpc_port: Some(settings.rpc_port),
        };
        let sink: Arc<dyn Sink> = sink.clone();
        let stop = Arc::clone(stop_pool);
        std::thread::spawn(move || pool::run(&options, sink, stop))
    };

    // The miner needs real work to do, not merely an open port.
    if !sink.wait_until_ready(|| pool.is_finished() || stop_pool.load(Ordering::Relaxed)) {
        controls.stop();
        return finish(pool.join(), Ok(()));
    }

    let miner = {
        let options = miner::Options {
            pool: settings.listen.to_string(),
            worker: worker_name(),
            // A thread per core, idle beyond the power level, so the
            // dashboard can turn the power up without reconnecting.
            slots: std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
            report_interval,
            lifetime_path: lifetime_path(settings.network),
        };
        let sink: Arc<dyn Sink> = sink.clone();
        let controls = Arc::clone(controls);
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

/// Ctrl-C (or a SIGTERM) asks both halves to finish, which lets the miner
/// save its lifetime totals. A second press means "now".
///
/// The dashboard reads Ctrl-C as a key, so there this only catches signals
/// sent from elsewhere, and stays quiet rather than print over the screen.
fn handle_signals(
    stop_pool: &Arc<AtomicBool>,
    controls: &Arc<Controls>,
    announce: bool,
) -> Result<(), String> {
    let stop_pool = Arc::clone(stop_pool);
    let controls = Arc::clone(controls);
    let presses = AtomicUsize::new(0);
    ctrlc::set_handler(move || {
        if presses.fetch_add(1, Ordering::Relaxed) > 0 {
            std::process::exit(130);
        }
        if announce {
            println!("\nstopping...");
        }
        stop_pool.store(true, Ordering::Relaxed);
        controls.stop();
    })
    .map_err(|error| format!("cannot handle Ctrl-C: {error}"))
}

/// What the session came to, once the dashboard has closed.
fn summarise(state: &dashboard::State) {
    println!(
        "mined for {} · {} hashes · best {} zero bits (a block needs {})",
        dashboard::format::uptime(state.now.saturating_sub(state.started), true),
        events::si(state.session_hashes),
        state.best_zero_bits,
        state.needed_zero_bits,
    );
    println!("the node is still running, so the next start is quick — `bip110-miner stop` shuts it down");
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

/// Passes events on, and notices when the pool has work.
struct Watch {
    output: Arc<dyn Sink>,
    ready: Mutex<bool>,
    changed: Condvar,
}

impl Watch {
    fn new(output: Arc<dyn Sink>) -> Self {
        Self { output, ready: Mutex::new(false), changed: Condvar::new() }
    }

    /// Waits for the pool's first job. False if `give_up` says to stop first.
    fn wait_until_ready(&self, give_up: impl Fn() -> bool) -> bool {
        let mut ready = self.ready.lock().expect("watch lock");
        while !*ready {
            if give_up() {
                return false;
            }
            ready = self.changed.wait_timeout(ready, Duration::from_millis(200)).expect("watch lock").0;
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
        self.output.emit(event);
    }
}
