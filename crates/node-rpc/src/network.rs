//! The three networks this fork runs, and where each keeps its data.
//!
//! # A hazard worth naming
//!
//! This fork **shares Bitcoin's P2P network and default ports**. It did not
//! change `pchMessageStart`, because the chains are identical up to the fork
//! height and a node upgrading across it should be able to keep its peers.
//!
//! The consequence is that a datadir mix-up does not fail loudly. Point a
//! BLAKE2b node at a Bitcoin datadir and it will start, connect, and quietly
//! disagree about which chain is real. Hence the dedicated `~/.btcb2`.

use std::path::{Path, PathBuf};

/// Which network a node is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// A private chain on this machine, where the fork height is ours to pick.
    Regtest,
    /// The public test network. BLAKE2b activates at height 150,308.
    Testnet,
    /// The real one. BLAKE2b activates at height 961,640.
    Mainnet,
}

impl Network {
    /// The default JSON-RPC port — Bitcoin's, unchanged.
    pub const fn default_rpc_port(self) -> u16 {
        match self {
            Self::Regtest => 18443,
            Self::Testnet => 18332,
            Self::Mainnet => 8332,
        }
    }

    /// The datadir subdirectory. Mainnet lives at the top level, historically.
    pub const fn datadir_subdirectory(self) -> Option<&'static str> {
        match self {
            Self::Regtest => Some("regtest"),
            Self::Testnet => Some("testnet3"),
            Self::Mainnet => None,
        }
    }

    /// Where the node writes its RPC cookie.
    pub fn cookie_path(self, datadir: &Path) -> PathBuf {
        match self.datadir_subdirectory() {
            Some(sub) => datadir.join(sub).join(".cookie"),
            None => datadir.join(".cookie"),
        }
    }

    /// The name `getblockchaininfo` reports in its `chain` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regtest => "regtest",
            Self::Testnet => "test",
            Self::Mainnet => "main",
        }
    }

    /// Parses the name used on the command line and by the RPC.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "regtest" => Some(Self::Regtest),
            "test" | "testnet" => Some(Self::Testnet),
            "main" | "mainnet" => Some(Self::Mainnet),
            _ => None,
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
