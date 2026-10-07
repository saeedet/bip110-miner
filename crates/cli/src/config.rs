//! Settings: one file, written by the program, never by hand.
//!
//! `~/.bip110-miner/config.toml` remembers the choices made on earlier runs —
//! which network, how hard to work, where rewards go — so later runs need no
//! arguments at all. Nobody should have to open it.
//!
//! # Where a setting comes from
//!
//! Highest wins:
//!
//! 1. a flag on the command line, for this run only;
//! 2. an environment variable, for scripts and the old shell tooling;
//! 3. the settings file;
//! 4. a sensible default.
//!
//! The environment variables and the old `payout.<network>` files are still
//! read, so a setup made before this file existed keeps working unchanged.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use node_rpc::Network;
use serde::{Deserialize, Serialize};

use crate::platform;

/// How hard to work.
///
/// Offered instead of a thread count, because "how many threads" means
/// nothing to most people and "how much heat and fan noise" means a lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Power {
    /// A quarter of the cores. Quiet, and the machine stays cool.
    Eco,
    /// All but two cores, so the machine stays pleasant to use.
    #[default]
    Balanced,
    /// Every core. Expect heat and fan noise.
    Max,
}

impl Power {
    /// How many hashing threads this setting means on a machine with `cores`.
    pub fn threads(self, cores: usize) -> usize {
        match self {
            Self::Eco => (cores / 4).max(1),
            Self::Balanced => cores.saturating_sub(2).max(1),
            Self::Max => cores.max(1),
        }
    }

    /// The setting's name, as shown and as written in the file.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Eco => "eco",
            Self::Balanced => "balanced",
            Self::Max => "max",
        }
    }
}

/// The settings file, exactly as stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct File {
    /// The network to mine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// How hard to work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<Power>,
    /// Where rewards go, per network: an address valid on one network is not
    /// valid on another.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub address: BTreeMap<String, String>,
    /// Where the node is and how to reach it, when not the defaults.
    #[serde(default, skip_serializing_if = "NodeSection::is_empty")]
    pub node: NodeSection,
}

/// The `[node]` section of the settings file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NodeSection {
    /// The directory holding `bitcoind` and `bitcoin-cli`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binaries: Option<PathBuf>,
    /// The node's data directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datadir: Option<PathBuf>,
    /// The node's RPC port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_port: Option<u16>,
}

impl NodeSection {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// The settings file's location.
pub fn path() -> PathBuf {
    platform::state_dir().join("config.toml")
}

impl File {
    /// Reads the settings, or an empty set if there is no file yet.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|error| format!("{} is not valid: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    /// Writes the settings, creating the directory if needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self).map_err(|error| error.to_string())?;
        let text = format!(
            "# Settings for bip110-miner, written by the program as you make choices.\n\
             # There is no need to edit this by hand.\n\n{body}"
        );
        std::fs::write(path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

/// Everything a command needs, with each value resolved from its sources.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The network to mine.
    pub network: Network,
    /// How hard to work.
    pub power: Power,
    /// Hashing threads, from `power` unless overridden.
    pub threads: usize,
    /// Whether `threads` was given exactly with `--threads`, not by power.
    pub threads_from_flag: bool,
    /// Where rewards go, if anywhere yet.
    pub address: Option<String>,
    /// Where the node's programs are.
    pub binaries: PathBuf,
    /// The node's data directory.
    pub datadir: PathBuf,
    /// The node's RPC port.
    pub rpc_port: u16,
    /// Where the pool listens for miners.
    pub listen: SocketAddr,
}

/// Overrides from the command line, for this run only.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// `--network`.
    pub network: Option<Network>,
    /// `--power`.
    pub power: Option<Power>,
    /// `--threads`.
    pub threads: Option<usize>,
}

impl Settings {
    /// Resolves every setting from flags, environment, file and defaults.
    pub fn resolve(file: &File, overrides: &Overrides) -> Result<Self, String> {
        let env = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        let network = match (overrides.network, &file.network) {
            (Some(network), _) => network,
            (None, Some(name)) => Network::parse(name)
                .ok_or_else(|| format!("the settings file names an unknown network {name:?}"))?,
            (None, None) => Network::Mainnet,
        };

        let power = overrides.power.or(file.power).unwrap_or_default();
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let threads = overrides.threads.unwrap_or_else(|| power.threads(cores)).max(1);

        let address = env("BIP110_PAYOUT_ADDRESS")
            .or_else(|| file.address.get(network_key(network)).cloned())
            .or_else(|| legacy_payout_address(network));

        let binaries = env("BIP110_KNOTS")
            .map(PathBuf::from)
            .or_else(|| file.node.binaries.clone())
            .unwrap_or_else(platform::default_node_binaries);

        let datadir = env("BIP110_DATADIR")
            .map(PathBuf::from)
            .or_else(|| file.node.datadir.clone())
            .unwrap_or_else(platform::default_node_datadir);

        let rpc_port = match env("BIP110_RPCPORT") {
            Some(port) => port.parse().map_err(|_| format!("BIP110_RPCPORT={port:?} is not a port"))?,
            None => file
                .node
                .rpc_port
                .or_else(|| crate::node::configured_rpc_port(network))
                .unwrap_or_else(|| network.default_rpc_port()),
        };

        Ok(Self {
            network,
            power,
            threads,
            threads_from_flag: overrides.threads.is_some(),
            address,
            binaries,
            datadir,
            rpc_port,
            listen: SocketAddr::from(([127, 0, 0, 1], 3334)),
        })
    }
}

/// The name a network goes by in the settings file.
pub const fn network_key(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet4 => "testnet4",
        Network::Regtest => "regtest",
    }
}

/// A payout address from the file the shell tooling used before settings
/// existed: `~/.bip110-miner/payout.<network>`.
fn legacy_payout_address(network: Network) -> Option<String> {
    let path = platform::state_dir().join(format!("payout.{}", network_key(network)));
    let address = std::fs::read_to_string(path).ok()?;
    let address = address.trim();
    (!address.is_empty()).then(|| address.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_maps_to_threads_on_an_eight_core_machine() {
        assert_eq!(Power::Eco.threads(8), 2);
        assert_eq!(Power::Balanced.threads(8), 6);
        assert_eq!(Power::Max.threads(8), 8);
    }

    /// Small machines must still mine on at least one thread.
    #[test]
    fn power_never_maps_to_zero_threads() {
        for power in [Power::Eco, Power::Balanced, Power::Max] {
            assert!(power.threads(1) >= 1, "{power:?} on one core");
            assert!(power.threads(2) >= 1, "{power:?} on two cores");
        }
    }

    #[test]
    fn the_file_round_trips() {
        let mut file = File {
            network: Some("mainnet".into()),
            power: Some(Power::Eco),
            ..File::default()
        };
        file.address.insert("mainnet".into(), "bc1qexample".into());
        file.node.rpc_port = Some(8342);

        let text = toml::to_string_pretty(&file).expect("serialises");
        assert_eq!(toml::from_str::<File>(&text).expect("parses"), file);
    }

    /// An empty file is a valid file, so a fresh install has no special case.
    #[test]
    fn an_empty_file_means_all_defaults() {
        let file: File = toml::from_str("").expect("parses");
        assert_eq!(file, File::default());
    }

    #[test]
    fn a_flag_beats_the_file() {
        let file = File { power: Some(Power::Max), ..File::default() };
        let overrides = Overrides { power: Some(Power::Eco), ..Overrides::default() };
        let settings = Settings::resolve(&file, &overrides).expect("resolves");
        assert_eq!(settings.power, Power::Eco);
    }

    #[test]
    fn an_unknown_network_in_the_file_is_an_error_not_a_guess() {
        let file = File { network: Some("moonnet".into()), ..File::default() };
        assert!(Settings::resolve(&file, &Overrides::default()).is_err());
    }
}
