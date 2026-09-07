//! A solo mining pool: a Knots node on one side, Stratum V1 on the other.
//!
//! ```text
//! pool [--network regtest] [--listen 127.0.0.1:3334] [--address bc1...]
//! ```
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
//! # Threads
//!
//! One poller watches the node for new templates. One thread accepts
//! connections. Each connection gets a reader and a writer. Nothing else.

mod job_builder;
mod min_difficulty;
mod readiness;
mod session;
mod state;
mod validate;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use btcb2_primitives::hex;
use node_rpc::{Network, RpcClient};
use stratum::{Request, method};

use state::PoolState;

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

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::from_args()?;

    let client = Arc::new(RpcClient::from_datadir(&options.datadir, options.network)?);

    // Every reason a node might be unfit to mine on lives in one place, so the
    // check cannot drift between callers. Notably it is not enough to ask
    // whether the node finished syncing: that answer is latched to false and
    // never revisited, so a node that later loses every peer still claims to be
    // caught up. See the `readiness` module.
    let info = client.get_blockchain_info()?;
    let peers = client.get_connection_count()?;
    readiness::check(options.network, &info, peers, unix_now())?;

    let payout_script = resolve_payout_script(&client, &options)?;
    let headline = chain::headline(options.network);

    println!("pool    : {} on {}", options.network, options.listen);
    println!("node    : height {} ({peers} peers)", info.blocks);
    println!("payout  : {}\n", hex::encode(&payout_script));

    let (state, wakeups) = PoolState::new();
    let state = Arc::new(state);

    // Bind before starting the poller. The poller announces readiness once it
    // has a job, and a supervising script may launch a miner the moment it sees
    // that — so the socket has to exist first, or the miner races the bind and
    // gets connection-refused.
    let listener = TcpListener::bind(options.listen)?;
    println!("waiting for miners on {}", options.listen);

    // The poller owns template watching; the main thread owns accepting.
    {
        let state = Arc::clone(&state);
        let client = Arc::clone(&client);
        let network = options.network;
        std::thread::spawn(move || {
            poll_templates(&state, &client, &payout_script, headline.as_deref(), &wakeups, network)
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // Small writes, sent immediately: a job notification delayed by
                // Nagle is a job the miner is not working on yet.
                let _ = stream.set_nodelay(true);

                let state = Arc::clone(&state);
                let client = Arc::clone(&client);
                std::thread::spawn(move || session::handle(stream, state, client));
            }
            Err(error) => eprintln!("cannot accept connection: {error}"),
        }
    }

    Ok(())
}

/// Watches the node for new work and pushes it to every connected miner.
fn poll_templates(
    state: &PoolState,
    client: &RpcClient,
    payout_script: &[u8],
    headline: Option<&[u8]>,
    wakeups: &std::sync::mpsc::Receiver<()>,
    network: Network,
) {
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

    loop {
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
                            eprintln!("cannot tell whether this is the fork block: {error}");
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
                                eprintln!("cannot read the parent header: {error}");
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
                                println!(
                                    "new tip at height {} — job {} ({transactions} txs, {} miners)",
                                    height - 1,
                                    job.job.job_id,
                                    state.subscriber_count(),
                                );

                                // Say plainly when the chain's clock is being
                                // pushed forward, and by how much. Mining the
                                // minimum-difficulty window this way is legal
                                // and standard on testnet4, but it is a choice
                                // that affects everyone on the network, so it
                                // should be visible rather than incidental.
                                if let Some(window) = window {
                                    let ahead = window.seconds_ahead_of(unix_now());
                                    let difficulty =
                                        btcb2_primitives::Target::difficulty(job.job.bits);
                                    if ahead > 0 {
                                        println!(
                                            "  minimum difficulty ({difficulty:.0}) — stamping \
                                             {}m{:02}s ahead of the wall clock",
                                            ahead / 60,
                                            ahead % 60,
                                        );
                                    } else {
                                        println!(
                                            "  minimum difficulty ({difficulty:.0}) — the window \
                                             is open on its own, no clock pushed"
                                        );
                                    }
                                }
                            } else if bits_changed {
                                println!(
                                    "DIFFICULTY CHANGED at height {height} — job {} \
                                     (difficulty {:.4}, {} miners)",
                                    job.job.job_id,
                                    btcb2_primitives::Target::difficulty(job.job.bits),
                                    state.subscriber_count(),
                                );
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
                                println!("POOL READY — first job built, serving miners");
                            }

                            last_tip = Some(tip);
                            last_bits = bits;
                            last_built = std::time::Instant::now();
                            last_job_installed = last_built;
                        }
                        Err(error) => eprintln!("cannot build a job: {error}"),
                    }
                }
            }
            Err(error) => eprintln!("cannot fetch a template: {error}"),
        }

        // Checked here rather than in the error arm above, so a run of *build*
        // failures counts as an outage too. Either way the miners are grinding
        // work that nothing has refreshed.
        if last_job_installed.elapsed() >= MAX_TEMPLATE_OUTAGE {
            eprintln!(
                "\nFATAL: no job installed for {}s. Miners would be hashing work that is \
                 probably already dead, so the pool is stopping rather than letting that \
                 continue silently.",
                last_job_installed.elapsed().as_secs(),
            );
            std::process::exit(1);
        }

        // Readiness is not a one-time property. A node whose peers are all
        // connected and useless will happily serve templates for a tip that
        // stopped moving hours ago; only re-checking the tip's age catches that.
        if last_readiness_check.elapsed() >= READINESS_INTERVAL {
            last_readiness_check = std::time::Instant::now();

            match (client.get_blockchain_info(), client.get_connection_count()) {
                (Ok(info), Ok(peers)) => {
                    if let Err(reason) = readiness::check(network, &info, peers, unix_now()) {
                        eprintln!("\nFATAL: the node is no longer fit to mine on — {reason}");
                        std::process::exit(1);
                    }
                }
                // A failure to ask is not a failure of the node; the outage
                // timer above is what handles an unreachable one.
                _ => eprintln!("cannot re-check node readiness"),
            }
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

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Works out where block rewards should go.
fn resolve_payout_script(
    client: &RpcClient,
    options: &Options,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // On regtest the coins are meaningless, so a throwaway wallet address is
    // fine and saves the user a step. Anywhere else, the address must be
    // supplied deliberately — this is real money and it should not default.
    let address = match &options.address {
        Some(address) => address.clone(),
        None if options.network == Network::Regtest => {
            const WALLET: &str = "btcb2-miner-regtest";
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

    println!("address : {address}");

    Ok(hex::decode(
        info.script_pubkey
            .as_deref()
            .ok_or("validateaddress returned no scriptPubKey")?,
    )?)
}

/// The one chain parameter a template does not carry.
mod chain {
    use node_rpc::Network;

    /// The headline the coinbase must carry at the activation height.
    ///
    /// `None` means the network sets none, and the rule cannot fail: the node
    /// searches the coinbase for an empty byte sequence, which always matches.
    pub fn headline(network: Network) -> Option<Vec<u8>> {
        match network {
            // Transcribed from `kernel/chainparams.cpp`, not paraphrased. One
            // byte wrong and the fork block is rejected as `bad-headline`.
            Network::Mainnet => Some(b"8-30 NYPost Deride And Conquer".to_vec()),
            Network::Testnet4 => None,
            Network::Regtest => Some(
                std::env::var("BTCB2_HEADLINE")
                    .unwrap_or_else(|_| "btcb2-miner regtest".to_owned())
                    .into_bytes(),
            ),
        }
    }
}

/// Command-line options.
struct Options {
    network: Network,
    listen: std::net::SocketAddr,
    address: Option<String>,
    datadir: PathBuf,
}

impl Options {
    fn from_args() -> Result<Self, Box<dyn std::error::Error>> {
        let mut network = Network::Regtest;
        // 3333 is Stratum's customary port and the sibling Bitcoin project takes
    // it. This chain already shares Bitcoin's P2P magic and RPC ports, which
    // is a hazard rather than a convenience; there is no reason to add a third
    // collision when the two miners might well run side by side.
    let mut listen = "127.0.0.1:3334".to_owned();
        let mut address = None;

        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));

            match flag.as_str() {
                "--network" => {
                    let name = value()?;
                    network =
                        Network::parse(&name).ok_or_else(|| format!("unknown network {name:?}"))?;
                }
                "--listen" => listen = value()?,
                "--address" => address = Some(value()?),
                "--help" | "-h" => {
                    println!(
                        "pool [--network regtest|testnet|mainnet] [--listen ADDR] [--address ADDR]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown option {other:?}").into()),
            }
        }

        Ok(Self {
            network,
            listen: listen.parse()?,
            address,
            datadir: std::env::var("BTCB2_DATADIR").map_or_else(
                |_| PathBuf::from(std::env::var("HOME").expect("HOME is set")).join(".btcb2"),
                PathBuf::from,
            ),
        })
    }
}
