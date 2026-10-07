//! A solo mining pool: a Knots node on one side, Stratum V1 on the other.
//!
//! # What a solo pool is for
//!
//! A normal pool exists to smooth income: hundreds of miners contribute work,
//! and the pool divides the reward. A *solo* pool divides nothing. It exists
//! because Stratum is the protocol mining hardware speaks, and speaking it is
//! what lets any miner point at a node without knowing anything about block
//! assembly.
//!
//! So the split is not ceremony. It is the seam that makes the hardware
//! question independent of the software one — and on this chain that seam is
//! unusually clean, because the proof of work was designed with it in mind. The
//! node's own source draws the line in the same place this pool does: stages 0
//! to 3 here, stages 4 and 5 on the miner.
//!
//! # A library, so it can share a process
//!
//! [`run`] is the whole pool. The `pool` binary wraps it with argument parsing
//! and plain output; the `bip110-miner` command runs it on a thread beside the
//! miner and a dashboard. Either way the Stratum port stays open, so other
//! hardware can still connect to it.
//!
//! Everything the pool has to say goes to a [`Sink`] as an [`Event`], never
//! straight to the terminal, and nothing in here ends the process: a fatal
//! condition is reported through the sink and returned as [`Reported`].
//!
//! # Threads
//!
//! One poller watches the node for new templates. One thread accepts
//! connections. Each connection gets a reader and a writer. Nothing else.

mod chain;
mod job_builder;
mod min_difficulty;
mod readiness;
mod session;
mod state;
mod validate;

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bip110_primitives::hex;
use events::{Event, Level, Sink};
use node_rpc::{Network, RpcClient};
use stratum::{Request, method};

use state::PoolState;

/// Errors from [`run`]. `Send + Sync` so a pool on its own thread can hand
/// its failure back to whoever started it.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// A failure that has already been reported through the sink.
///
/// Returned so the caller knows to stop, and knows not to print the reason a
/// second time.
#[derive(Debug)]
pub struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the pool stopped; the reason was reported above")
    }
}

impl std::error::Error for Reported {}

/// How the pool should run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Which network the node is on.
    pub network: Network,
    /// Where to listen for miners.
    pub listen: SocketAddr,
    /// Where rewards go. Required everywhere except regtest, where a throwaway
    /// wallet address is generated.
    pub address: Option<String>,
    /// The node's data directory, for its RPC cookie.
    pub datadir: PathBuf,
    /// The node's RPC port, when it is not the network default.
    pub rpc_port: Option<u16>,
}

/// Runs the pool until `stop` is set or something fatal happens.
///
/// Blocks the calling thread. Waits for the node to be fit to mine on before
/// opening the Stratum port, so a miner that connects is never handed work
/// built on a stale tip.
pub fn run(options: &Options, sink: Arc<dyn Sink>, stop: Arc<AtomicBool>) -> Result<(), Error> {
    let client = Arc::new(match options.rpc_port {
        Some(port) => RpcClient::connect(&options.datadir, options.network, port)?,
        None => RpcClient::from_datadir(&options.datadir, options.network)?,
    });

    // Every reason a node might be unfit to mine on lives in one place, so the
    // check cannot drift between callers. Notably it is not enough to ask
    // whether the node finished syncing: that answer is latched to false and
    // never revisited, so a node that later loses every peer still claims to be
    // caught up. See the `readiness` module.
    let Some((info, peers)) = wait_until_ready(&client, options.network, sink.as_ref(), &stop)?
    else {
        return Ok(());
    };

    let (address, payout_script) = resolve_payout_script(&client, options)?;
    let headline = chain::headline(options.network);

    sink.emit(Event::PoolStarted {
        address,
        payout_script: hex::encode(&payout_script),
        network: options.network.to_string(),
        listen: options.listen.to_string(),
        height: info.blocks,
        peers,
    });

    let (state, wakeups) = PoolState::new();
    let state = Arc::new(state);

    // Bind before starting the poller. The poller announces readiness once it
    // has a job, and a supervisor may launch a miner the moment it sees that —
    // so the socket has to exist first, or the miner races the bind and gets
    // connection-refused.
    let listener = TcpListener::bind(options.listen)?;
    // Non-blocking, so the accept loop can notice `stop`. A blocking accept
    // waits for the next miner indefinitely, which made the pool impossible to
    // stop except by ending the whole process.
    listener.set_nonblocking(true)?;
    sink.emit(Event::Listening { listen: options.listen.to_string() });

    let poller = {
        let state = Arc::clone(&state);
        let client = Arc::clone(&client);
        let sink = Arc::clone(&sink);
        let stop = Arc::clone(&stop);
        let network = options.network;
        std::thread::spawn(move || {
            poll_templates(
                &state,
                &client,
                &payout_script,
                headline.as_deref(),
                &wakeups,
                network,
                sink.as_ref(),
                &stop,
            )
        })
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // The poller only returns early on a fatal condition, which it has
        // already reported. Hand that back rather than serving a dead job.
        if poller.is_finished() {
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit non-blocking mode on macOS and the
                // BSDs, and the session code expects blocking reads.
                let _ = stream.set_nonblocking(false);
                // Small writes, sent immediately: a job notification delayed by
                // Nagle is a job the miner is not working on yet.
                let _ = stream.set_nodelay(true);

                let state = Arc::clone(&state);
                let client = Arc::clone(&client);
                let sink = Arc::clone(&sink);
                std::thread::spawn(move || session::handle(stream, state, client, sink));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => warn(sink.as_ref(), format!("cannot accept connection: {error}")),
        }
    }

    stop.store(true, Ordering::Relaxed);
    poller.join().unwrap_or_else(|_| Err("the template poller panicked".into()))
}

/// How often to ask the node whether the work has changed.
///
/// Polling is the simple answer, and at this interval it costs nothing on a
/// local node. The better answer is the ZMQ `hashblock` notification the
/// regtest config already enables, which would cut the latency between a new
/// block arriving and miners being told from half a second to nearly zero.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How often to rebuild the job even when the tip has not moved, to pick up
/// transactions that arrived in the meantime.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// How long the node may fail to produce a template before we give up.
///
/// Carrying on regardless is the worst option: miners keep hashing the last
/// job, which quietly becomes worthless the moment the tip moves, and the
/// operator sees a healthy hashrate the whole time.
const MAX_TEMPLATE_OUTAGE: Duration = Duration::from_secs(120);

/// How often to re-examine whether the node is still worth mining on.
const READINESS_INTERVAL: Duration = Duration::from_secs(60);

/// How long the node may be unfit to mine on before the pool gives up.
///
/// Unreadiness is usually **temporary**: a node that has just started, or that
/// briefly fell behind its own headers, is catching up and will be fine in
/// seconds or minutes. Treating the first failed check as fatal — which this
/// pool used to — turned every such moment into a stopped miner needing a
/// manual restart.
///
/// So a failed check now pauses job building and keeps watching. Half an hour
/// is long enough to ride out a restart catching up on a few hundred blocks,
/// and short enough that a genuinely broken node does not leave miners grinding
/// a dead job all night.
const MAX_UNREADY: Duration = Duration::from_secs(30 * 60);

/// Watches the node for new work and pushes it to every connected miner.
///
/// Returns when `stop` is set, or with [`Reported`] when the node has been
/// unusable for long enough that mining on can only waste work.
#[allow(clippy::too_many_arguments)]
fn poll_templates(
    state: &PoolState,
    client: &RpcClient,
    payout_script: &[u8],
    headline: Option<&[u8]>,
    wakeups: &std::sync::mpsc::Receiver<()>,
    network: Network,
    sink: &dyn Sink,
    stop: &AtomicBool,
) -> Result<(), Error> {
    // testnet4 is the only network here with a minimum-difficulty rule.
    // Mainnet has none, and on regtest the target is trivial already.
    let min_difficulty_network = network == Network::Testnet4;

    let mut last_tip = None;
    let mut last_bits: Option<u32> = None;
    let mut last_built = std::time::Instant::now();
    // Health is measured by the last job we actually *installed*, not the last
    // RPC that answered. A node can serve templates perfectly while every one
    // of them fails to become a job — a witness-commitment disagreement, say —
    // and in that state the previously installed job is stale the moment the
    // tip moves. Timing from the RPC would call that healthy.
    let mut last_job_installed = std::time::Instant::now();
    let mut last_readiness_check = std::time::Instant::now();
    let mut announced_ready = false;
    // When the node first became unfit, and whether we have said so. `None`
    // means fit.
    let mut unready_since: Option<std::time::Instant> = None;
    let mut announced_unready = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Paused: the node is not fit to mine on, so a template from it would
        // build on a parent the network has moved past. Skip the fetch entirely
        // rather than hand miners work that is known to be wasted.
        if unready_since.is_some() {
            if wakeups.recv_timeout(POLL_INTERVAL).is_ok() {
                while wakeups.try_recv().is_ok() {}
            }
            recheck_readiness(
                client,
                network,
                sink,
                &mut last_readiness_check,
                &mut unready_since,
                &mut announced_unready,
            )?;
            continue;
        }

        match client.get_block_template() {
            Ok(template) => {
                let tip = template.previous_block_hash.clone();

                // A changed tip means everything in flight is now worthless, so
                // miners are told to discard it. A periodic refresh on the same
                // tip only adds transactions, so old work stays valid.
                let tip_changed = last_tip.as_ref() != Some(&tip);

                // The difficulty can change without the tip moving — on a
                // network with the minimum-difficulty rule, or across the fork
                // height, where the node applies a one-off target shift. Either
                // way the work in flight is aimed at the wrong target.
                let bits = template.compact_bits().ok();
                let bits_changed = last_bits.is_some() && bits.is_some() && bits != last_bits;

                let stale = last_built.elapsed() >= REFRESH_INTERVAL;

                if tip_changed || bits_changed || stale {
                    let job_id = state.allocate_job_id();

                    // The headline is required at exactly the activation height
                    // and nowhere else. `is_fork_block` costs one RPC, and only
                    // when the template is for a v2 block at all.
                    let headline = match (headline, is_fork_block(client, &template)) {
                        (Some(headline), Ok(true)) => Some(headline),
                        (_, Err(error)) => {
                            warn(sink, format!("cannot tell whether this is the fork block: {error}"));
                            None
                        }
                        _ => None,
                    };

                    // Both a new tip and a difficulty change invalidate work in
                    // flight, so miners are told to start over in either case.
                    let clean = tip_changed || bits_changed;

                    // The template's own timestamp and target are the wrong
                    // ones on a minimum-difficulty network — see the
                    // `min_difficulty` module. Computing the window needs the
                    // parent's timestamp, which the template does not carry.
                    let window = if min_difficulty_network {
                        match client.get_block_header(&template.previous_block_hash) {
                            Ok(parent) => {
                                let min_time = u32::try_from(template.min_time).unwrap_or(0);
                                Some(min_difficulty::plan(parent.time, min_time))
                            }
                            Err(error) => {
                                warn(sink, format!("cannot read the parent header: {error}"));
                                None
                            }
                        }
                    } else {
                        None
                    };

                    match job_builder::build(
                        job_id, &template, payout_script, headline, window, clean,
                    ) {
                        Ok(active) => {
                            let height = active.height;
                            let transactions = active.block.transactions.len();
                            let job = state.set_current_job(active);

                            if tip_changed {
                                sink.emit(Event::NewJob {
                                    height,
                                    job_id: job.job.job_id.clone(),
                                    transactions,
                                    miners: state.subscriber_count(),
                                    reward: template.coinbase_value,
                                });

                                // Say plainly when the chain's clock is being
                                // pushed forward, and by how much. Mining the
                                // minimum-difficulty window this way is legal
                                // and standard on testnet4, but it is a choice
                                // that affects everyone on the network, so it
                                // should be visible rather than incidental.
                                if let Some(window) = window {
                                    sink.emit(Event::MinimumDifficulty {
                                        difficulty: bip110_primitives::Target::difficulty(
                                            job.job.bits,
                                        ),
                                        seconds_ahead: window.seconds_ahead_of(unix_now()),
                                    });
                                }
                            } else if bits_changed {
                                sink.emit(Event::DifficultyChanged {
                                    height,
                                    job_id: job.job.job_id.clone(),
                                    difficulty: bip110_primitives::Target::difficulty(job.job.bits),
                                    miners: state.subscriber_count(),
                                });
                            }

                            let notify =
                                Request::notification(method::NOTIFY, job.job.to_notify_params());
                            if let Ok(line) = serde_json::to_string(&notify) {
                                state.broadcast(&line);
                            }

                            // A readiness signal, so a supervising script can
                            // wait for real work rather than guessing from a
                            // sleep and a liveness check.
                            if !announced_ready {
                                announced_ready = true;
                                sink.emit(Event::PoolReady);
                            }

                            last_tip = Some(tip);
                            last_bits = bits;
                            last_built = std::time::Instant::now();
                            last_job_installed = last_built;
                        }
                        Err(error) => warn(sink, format!("cannot build a job: {error}")),
                    }
                }
            }
            Err(error) => warn(sink, format!("cannot fetch a template: {error}")),
        }

        // Checked here rather than in the error arm above, so a run of *build*
        // failures counts as an outage too. Either way the miners are grinding
        // work that nothing has refreshed.
        if last_job_installed.elapsed() >= MAX_TEMPLATE_OUTAGE {
            return fatal(
                sink,
                format!(
                    "\nFATAL: no job installed for {}s. Miners would be hashing work that is \
                     probably already dead, so the pool is stopping rather than letting that \
                     continue silently.",
                    last_job_installed.elapsed().as_secs(),
                ),
            );
        }

        // Readiness is not a one-time property. A node whose peers are all
        // connected and useless will happily serve templates for a tip that
        // stopped moving hours ago; only re-checking the tip's age catches that.
        recheck_readiness(
            client,
            network,
            sink,
            &mut last_readiness_check,
            &mut unready_since,
            &mut announced_unready,
        )?;

        // While paused, do not let the template-outage timer fire. It exists to
        // catch a node that has silently stopped serving work; a node we have
        // deliberately stopped asking is not that, and letting it trip here
        // would reintroduce the very exit this pause replaced.
        if unready_since.is_some() {
            last_job_installed = std::time::Instant::now();
        }

        // Sleep, but wake early if a session tells us the tip moved. Draining
        // any further requests that piled up keeps one burst of accepted blocks
        // from causing a burst of redundant template fetches.
        if wakeups.recv_timeout(POLL_INTERVAL).is_ok() {
            while wakeups.try_recv().is_ok() {}
        }
    }
}

/// Whether this template is for the block at exactly the activation height.
///
/// There is no field for it. The template says whether *this* block needs a v2
/// header; the fork block is the one where that is true and it was not true for
/// the parent, so the parent's header is what has to be looked at.
fn is_fork_block(
    client: &RpcClient,
    template: &node_rpc::BlockTemplate,
) -> Result<bool, node_rpc::RpcError> {
    if !template.header_v2() {
        return Ok(false);
    }

    let parent = client.get_block_header(&template.previous_block_hash)?;
    Ok(!parent.is_header_v2())
}

/// Blocks until the node is fit to mine on, or gives up after [`MAX_UNREADY`].
///
/// A node that has just been started — which `scripts/mine.sh` does — has its
/// headers long before it has the blocks under them, so it is briefly unfit
/// through no fault of its own. Failing immediately there made starting the
/// miner a coin toss; waiting makes it deterministic.
///
/// Returns `None` if `stop` is set while waiting.
fn wait_until_ready(
    client: &RpcClient,
    network: Network,
    sink: &dyn Sink,
    stop: &AtomicBool,
) -> Result<Option<(node_rpc::BlockchainInfo, u32)>, Error> {
    let started = std::time::Instant::now();
    let mut announced = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let info = client.get_blockchain_info()?;
        let peers = client.get_connection_count()?;

        match readiness::check(network, &info, peers, unix_now()) {
            Ok(()) => return Ok(Some((info, peers))),
            Err(reason) => {
                if started.elapsed() >= MAX_UNREADY {
                    return Err(format!(
                        "the node was still unfit to mine on after {} minutes — {reason}",
                        started.elapsed().as_secs() / 60
                    )
                    .into());
                }
                if !announced {
                    announced = true;
                    sink.emit(Event::WaitingForNode { reason: reason.to_string() });
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Re-checks readiness on a schedule, pausing or resuming as it changes.
///
/// Shared by the paused branch and the normal path so the two cannot disagree
/// about when a pause starts, ends, or becomes fatal.
fn recheck_readiness(
    client: &RpcClient,
    network: Network,
    sink: &dyn Sink,
    last_check: &mut std::time::Instant,
    unready_since: &mut Option<std::time::Instant>,
    announced_unready: &mut bool,
) -> Result<(), Error> {
    if last_check.elapsed() < READINESS_INTERVAL {
        return Ok(());
    }
    *last_check = std::time::Instant::now();

    let (Ok(info), Ok(peers)) = (client.get_blockchain_info(), client.get_connection_count())
    else {
        // A failure to ask is not a failure of the node; the template-outage
        // timer is what handles an unreachable one.
        warn(sink, "cannot re-check node readiness".to_owned());
        return Ok(());
    };

    match readiness::check(network, &info, peers, unix_now()) {
        Err(reason) => {
            let since = unready_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= MAX_UNREADY {
                return fatal(
                    sink,
                    format!(
                        "\nFATAL: the node has been unfit to mine on for {} minutes — {reason}",
                        since.elapsed().as_secs() / 60,
                    ),
                );
            }
            // Said once per spell, not once per check, so a long catch-up does
            // not bury the log.
            if !*announced_unready {
                *announced_unready = true;
                sink.emit(Event::Paused { reason: reason.to_string() });
            }
        }
        Ok(()) => {
            if *announced_unready {
                sink.emit(Event::Resumed);
            }
            *announced_unready = false;
            *unready_since = None;
        }
    }
    Ok(())
}

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Works out where block rewards should go: the address and its script.
fn resolve_payout_script(client: &RpcClient, options: &Options) -> Result<(String, Vec<u8>), Error> {
    // On regtest the coins are meaningless, so a throwaway wallet address is
    // fine and saves the user a step. Anywhere else, the address must be
    // supplied deliberately — this is real money and it should not default.
    let address = match &options.address {
        Some(address) => address.clone(),
        None if options.network == Network::Regtest => {
            const WALLET: &str = "bip110-miner-regtest";
            client.ensure_wallet(WALLET)?;
            client.get_new_address(WALLET)?
        }
        None => {
            return Err(format!(
                "--address is required on {}. Rewards are paid to it and cannot be recovered \
                 if it is wrong, so there is deliberately no default.",
                options.network
            )
            .into());
        }
    };

    // The safety gate. A wrong-network or mistyped address does not fail
    // loudly at mining time — it produces a perfectly valid block paying to
    // nothing anyone can spend.
    //
    // It cannot catch everything here. This chain kept Bitcoin's address
    // format, so a Bitcoin address validates on this network and vice versa.
    // What this proves is that the address is well-formed for the network; it
    // cannot prove you meant to mine this chain.
    let info = client.validate_address(&address)?;
    if !info.is_valid {
        return Err(format!(
            "{address:?} is not a valid {} address — refusing to mine to it",
            options.network
        )
        .into());
    }

    let script = hex::decode(
        info.script_pubkey
            .as_deref()
            .ok_or("validateaddress returned no scriptPubKey")?,
    )?;
    Ok((address, script))
}


/// Reports a warning that does not stop the pool.
fn warn(sink: &dyn Sink, text: String) {
    sink.emit(Event::Log { level: Level::Warn, text });
}

/// Reports a fatal condition and returns the error that stops the pool.
fn fatal(sink: &dyn Sink, text: String) -> Result<(), Error> {
    sink.emit(Event::Log { level: Level::Fatal, text });
    Err(Box::new(Reported))
}
