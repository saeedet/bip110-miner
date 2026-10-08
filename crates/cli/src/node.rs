//! The node: finding it, starting and stopping it, and checking it is new
//! enough to mine with.
//!
//! The node is a separate long-lived program, Bitcoin Knots. It deliberately
//! outlives a mining session — stopping it would drop its peers and let it fall
//! behind, so the next start would pay to catch up — which is why mining stops
//! on `q` but the node only stops on `bip110-miner stop`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use node_rpc::{Network, RpcClient};

use crate::config::{Settings, network_key};
use crate::platform;

/// The oldest Knots this program will mine with: 29.4.2.
///
/// 29.4.2 activated a soft fork at block 973,440 under which a coinbase must
/// wait 6,480 blocks, about 45 days, before it can be spent. An older node
/// doesn't know the rule, so its block templates can include spends the
/// network now rejects — and a block found on one of those templates is
/// invalid. Refusing to mine is the only safe response.
pub const MIN_KNOTS: (u32, u32, u32) = (29, 4, 2);

/// The minimum under the name shared code uses, whichever node this is.
pub use self::MIN_KNOTS as MIN_VERSION;

/// The node's configuration for each network, compiled in so an installed
/// binary does not depend on the source tree.
fn template(network: Network) -> &'static str {
    match network {
        Network::Mainnet => include_str!("../../../config/bip110.mainnet.conf"),
        Network::Testnet4 => include_str!("../../../config/bip110.testnet4.conf"),
        Network::Regtest => include_str!("../../../config/bip110.regtest.conf"),
    }
}

/// The RPC port the node's configuration sets, if it moves it.
///
/// Mainnet's moves it to 8342, off Bitcoin's 8332, so this and a Bitcoin node
/// can run side by side.
pub fn configured_rpc_port(network: Network) -> Option<u16> {
    template(network)
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("rpcport="))
        .and_then(|port| port.trim().parse().ok())
}

/// Where the node's configuration file is written.
pub fn conf_path(network: Network) -> PathBuf {
    platform::state_dir().join("node").join(format!("{}.conf", network_key(network)))
}

/// Writes the node's configuration file, replacing any older copy.
///
/// The file belongs to this program: a newer release may carry different
/// settings — fresh peers to connect to, say — and they should take effect.
fn write_conf(network: Network) -> Result<PathBuf, String> {
    let path = conf_path(network);
    let wanted = template(network);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(wanted) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, wanted).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(path)
}

/// An RPC client for the node, if it is running.
pub fn client(settings: &Settings) -> Result<RpcClient, String> {
    RpcClient::connect(&settings.datadir, settings.network, settings.rpc_port)
        .map_err(|error| error.to_string())
}

/// Whether the node is up and answering.
pub fn is_running(settings: &Settings) -> bool {
    client(settings)
        .and_then(|client| client.get_blockchain_info().map_err(|error| error.to_string()))
        .is_ok()
}

/// The installed Knots version, read from the binary itself.
pub fn installed_version(settings: &Settings) -> Result<(u32, u32, u32), String> {
    let daemon = settings.binaries.join(platform::node_daemon());
    let output = Command::new(&daemon)
        .arg("-version")
        .output()
        .map_err(|_| format!("Bitcoin Knots is not installed at {}", settings.binaries.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_version(&text).ok_or_else(|| format!("cannot tell which version {} is", daemon.display()))
}

/// Pulls `29.4.2` out of `Bitcoin Knots daemon version v29.4.2.knots20260508`.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    // A `v` followed by a digit — not merely a `v`, or the word "version"
    // earlier on the same line would match first.
    let after = text
        .split_whitespace()
        .filter_map(|word| word.strip_prefix('v'))
        .find(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut numbers = after.split(['.', '-']).map(|part| part.parse::<u32>().ok());
    Some((numbers.next()??, numbers.next()??, numbers.next().flatten().unwrap_or(0)))
}

/// The version of the node that is actually running, from its user agent.
///
/// Distinct from [`installed_version`]: a node started earlier, perhaps from
/// another build, keeps running the old code after a newer one is installed.
/// What matters for mining is the one answering RPC.
pub fn running_version(client: &RpcClient) -> Option<(u32, u32, u32)> {
    let info: serde_json::Value = client.call("getnetworkinfo", serde_json::json!([])).ok()?;
    let agent = info.get("subversion")?.as_str()?;
    // "/Satoshi:29.4.2/Knots:20260508/" — the Core-compatible part carries it.
    let version = agent.split('/').find_map(|part| part.strip_prefix("Satoshi:"))?;
    parse_version(&format!("v{version}"))
}

/// Formats a version triple as `29.4.2`.
pub fn version_string((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// Starts the node and waits until it answers.
///
/// Loading a large chainstate takes minutes, so this waits patiently, calling
/// `waiting` once a second so the caller can show that something is happening.
pub fn start(settings: &Settings, mut waiting: impl FnMut(Duration)) -> Result<(), String> {
    let version = installed_version(settings)?;
    if version < MIN_KNOTS {
        return Err(format!(
            "Bitcoin Knots {} is too old to mine with: {} or newer is needed, because \
             earlier versions don't know the rule change at block 973,440",
            version_string(version),
            version_string(MIN_KNOTS),
        ));
    }

    let conf = write_conf(settings.network)?;
    std::fs::create_dir_all(&settings.datadir)
        .map_err(|error| format!("cannot create {}: {error}", settings.datadir.display()))?;

    let output = Command::new(settings.binaries.join(platform::node_daemon()))
        .arg(format!("-datadir={}", settings.datadir.display()))
        .arg(format!("-conf={}", conf.display()))
        // Wins over the config file's own port, so a port moved because
        // something else held the usual one takes effect.
        .arg(format!("-rpcport={}", settings.rpc_port))
        .arg("-daemon")
        .output()
        .map_err(|error| format!("cannot start the node: {error}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = if said.trim().is_empty() { String::from_utf8_lossy(&output.stdout) } else { said };
        return Err(format!("the node would not start: {}", said.trim()));
    }

    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(600) {
        if is_running(settings) {
            return Ok(());
        }
        waiting(started.elapsed());
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(format!("the node did not answer within ten minutes — its log is at {}", settings.datadir.display()))
}

/// Asks the node to shut down.
pub fn stop(settings: &Settings) -> Result<(), String> {
    let client = client(settings)?;
    client
        .call::<serde_json::Value>("stop", serde_json::json!([]))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_knots_prints() {
        assert_eq!(
            parse_version("Bitcoin Knots daemon version v29.4.2.knots20260508\nCopyright..."),
            Some((29, 4, 2))
        );
        assert_eq!(parse_version("Bitcoin Knots daemon version v29.4.knots20260508"), Some((29, 4, 0)));
        assert_eq!(parse_version("Bitcoin Knots daemon version v30.0"), Some((30, 0, 0)));
        assert_eq!(parse_version("nothing useful"), None);
    }

    /// The release that must be refused, and the one that must be accepted.
    #[test]
    fn reads_a_user_agent_version() {
        let version = "/Satoshi:29.4.2/Knots:20260508/"
            .split('/')
            .find_map(|part| part.strip_prefix("Satoshi:"))
            .and_then(|v| parse_version(&format!("v{v}")));
        assert_eq!(version, Some((29, 4, 2)));
    }

    #[test]
    fn the_minimum_is_the_soft_fork_release() {
        assert!((29, 4, 1) < MIN_KNOTS);
        assert!((29, 4, 2) >= MIN_KNOTS);
        assert!((30, 0, 0) >= MIN_KNOTS);
    }

    /// Mainnet's node moves its RPC port off Bitcoin's, and the program must
    /// know where it went.
    #[test]
    fn knows_where_each_network_listens_for_rpc() {
        assert_eq!(configured_rpc_port(Network::Mainnet), Some(8342));
        assert_eq!(configured_rpc_port(Network::Testnet4), Some(48342));
        assert_eq!(configured_rpc_port(Network::Regtest), Some(18453));
    }
}
