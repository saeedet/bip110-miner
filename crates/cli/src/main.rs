//! `bip110-miner` — mine the BIP-110 fork of Bitcoin from this machine.
//!
//! One command. Run it with nothing after it to mine: it starts the node if
//! needed, then the pool and the miner, and stops them on Ctrl-C. The other
//! commands look things up or change settings.
//!
//! Honest version up front: a found block is a lottery win, not a paycheck.

mod chain;
mod commands;
mod config;
mod dashboard;
mod node;
mod platform;
mod setup;

use clap::{Parser, Subcommand};
use node_rpc::Network;

use config::{Overrides, Power, Settings};

/// Mine the BIP-110 fork of Bitcoin from this machine.
///
/// Run with no command to start mining. Finding a block is a lottery win, not
/// a paycheck: expect to mine for a very long time and find nothing.
#[derive(Parser)]
#[command(name = "bip110-miner", version, about, long_about = None)]
struct Cli {
    /// Which network: mainnet, testnet4 or regtest [default: from settings, or mainnet]
    #[arg(long, global = true, value_parser = parse_network)]
    network: Option<Network>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Mine (what running bip110-miner on its own does)
    Start {
        /// How hard to work: eco, balanced or max
        #[arg(long)]
        power: Option<Power>,
        /// Exact number of hashing threads, instead of a power level
        #[arg(long, conflicts_with = "power")]
        threads: Option<usize>,
        /// Let the machine sleep while mining
        #[arg(long)]
        allow_sleep: bool,
        /// Print a log instead of the live dashboard
        #[arg(long)]
        plain: bool,
    },
    /// Save settings, such as where rewards go
    Setup {
        /// Where rewards are paid on this network
        #[arg(long)]
        address: Option<String>,
        /// How hard to work: eco, balanced or max
        #[arg(long)]
        power: Option<Power>,
    },
    /// Show where the node and the chain are
    Status,
    /// Show where rewards go, and whether this node can spend them
    Wallet,
    /// Check everything mining depends on
    Doctor,
    /// Shut the node down (it keeps running after mining stops, so starts are quick)
    Stop,
}

fn parse_network(name: &str) -> Result<Network, String> {
    Network::parse(name).ok_or_else(|| format!("unknown network {name:?} (mainnet, testnet4, regtest)"))
}

fn main() {
    let cli = Cli::parse();

    if let Err(message) = run(cli) {
        // An empty message means the problem was already reported as it
        // happened, in more detail than could be repeated here.
        if !message.is_empty() {
            eprintln!("error: {message}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let file = config::File::load(&config::path())?;

    let (power, threads) = match &cli.command {
        Some(Command::Start { power, threads, .. }) => (*power, *threads),
        _ => (None, None),
    };
    let overrides = Overrides { network: cli.network, power, threads };
    let settings = Settings::resolve(&file, &overrides)?;

    match cli.command {
        None => commands::start::run(&settings, &commands::start::Options { keep_awake: true, plain: false }),
        Some(Command::Start { allow_sleep, plain, .. }) => {
            commands::start::run(&settings, &commands::start::Options { keep_awake: !allow_sleep, plain })
        }
        Some(Command::Setup { address, power }) => commands::setup::run(
            &commands::setup::Changes {
                network: settings.network,
                make_default: cli.network.is_some(),
                address,
                power,
            },
            &settings,
        ),
        Some(Command::Status) => commands::status::run(&settings),
        Some(Command::Wallet) => commands::wallet::run(&settings),
        Some(Command::Doctor) => commands::doctor::run(&settings),
        Some(Command::Stop) => commands::stop::run(&settings),
    }
}
