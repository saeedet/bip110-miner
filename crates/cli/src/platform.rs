//! Everything that assumes macOS, in one place.
//!
//! The assumptions: where files live, how the node binary is found, how to
//! describe the machine and its free disk, how to download and unpack, and how
//! to stop the machine sleeping while it mines. Keeping them here rather than
//! scattered through the commands means a Linux or Windows port is a matter of
//! filling in this file, not of hunting for every path built by hand elsewhere.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The user's home directory.
pub fn home() -> PathBuf {
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

/// This machine in a few words: `Apple M3 Pro · 12 cores`.
pub fn machine() -> String {
    let chip = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "this computer".to_owned());
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    format!("{chip} · {cores} cores")
}

/// Free space on the disk holding `path`, in bytes.
///
/// `path` need not exist yet; the nearest directory above it that does is
/// asked instead, since that is the disk it will be created on.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|dir| dir.exists())?;
    let output = Command::new("df").arg("-k").arg(existing).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    // Filesystem 1024-blocks Used Available ...
    let available: u64 = text.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(available * 1024)
}

/// Starts downloading `url` to `destination`, carrying on from where an
/// earlier attempt stopped if part of the file is already there.
///
/// Uses the system's `curl`, which every Mac has, rather than adding a TLS
/// stack to this program. The caller watches the file grow for progress and
/// waits on the returned process for the outcome.
pub fn start_download(url: &str, destination: &Path) -> std::io::Result<Child> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Command::new("curl")
        // --fail: an error page is an error, not a file. -C -: resume.
        .args(["--fail", "--location", "--silent", "--show-error", "--retry", "3", "-C", "-", "-o"])
        .arg(destination)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

/// Unpacks a `.tar.gz` into `directory`.
pub fn unpack(archive: &Path, directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(directory)
        .output()
        .map_err(|error| format!("cannot run tar: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("cannot unpack {}: {}", archive.display(), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

/// Whether something on this machine is listening on `port`.
pub fn port_in_use(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
}

/// The first port from `start` that nothing is listening on and that can be
/// bound, trying a handful before giving up.
pub fn free_port_from(start: u16) -> Option<u16> {
    (start..start.saturating_add(20)).find(|&port| {
        !port_in_use(port) && std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
    })
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
