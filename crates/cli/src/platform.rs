//! Everything that assumes macOS, in one place.
//!
//! There are three such assumptions: where files live, how the node binary is
//! found, and how to stop the machine sleeping while it mines. Keeping them
//! here rather than scattered through the commands means a Linux or Windows
//! port is a matter of filling in this file, not of hunting for every path
//! built by hand elsewhere.

use std::path::PathBuf;
use std::process::{Child, Command};

/// The user's home directory.
fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Where this program keeps its own state: settings, lifetime totals, and the
/// node software it installs.
pub fn state_dir() -> PathBuf {
    home().join(".bip110-miner")
}

/// The node's data directory, unless the settings say otherwise.
///
/// Deliberately not Bitcoin Knots' own default. This fork shares Bitcoin's
/// network magic, so a node pointed at a Bitcoin data directory would start,
/// connect, and quietly disagree about which chain is real.
pub fn default_node_datadir() -> PathBuf {
    home().join(".bip110")
}

/// Where the node's programs live, unless the settings say otherwise.
pub fn default_node_binaries() -> PathBuf {
    state_dir().join("knots").join("current").join("bin")
}

/// The node daemon's file name.
pub const fn node_daemon() -> &'static str {
    if cfg!(windows) { "bitcoind.exe" } else { "bitcoind" }
}

/// Keeps the machine from sleeping for as long as this value lives.
///
/// A laptop that idles to sleep stops mining without a word, which a newcomer
/// would reasonably read as the program having crashed. On macOS this runs the
/// system's own `caffeinate`, tied to this process so it cannot outlive it.
pub struct KeepAwake(Option<Child>);

impl KeepAwake {
    /// Starts preventing sleep. Does nothing where that isn't supported yet.
    pub fn start() -> Self {
        if cfg!(target_os = "macos") {
            // -i: no idle sleep. -w: exit when this process does, even if it
            // is killed rather than stopped cleanly.
            let child = Command::new("caffeinate")
                .arg("-i")
                .arg("-w")
                .arg(std::process::id().to_string())
                .spawn()
                .ok();
            Self(child)
        } else {
            Self(None)
        }
    }

    /// Whether sleep is actually being prevented.
    pub fn active(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
