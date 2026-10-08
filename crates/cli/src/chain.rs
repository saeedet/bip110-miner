//! What this program's chain does differently from its sibling's.
//!
//! The dashboard, the commands and the layout are shared with btc-miner. The
//! handful of facts that are not — the node's name, how long a reward takes
//! to mature, which peers count as being on the right network — live here,
//! so the two projects differ in this file and not all over.

use node_rpc::{Network, RpcClient};
use serde_json::{Value, json};

use crate::setup::address::Way;

/// The program's name, as typed and as shown.
pub const PROGRAM: &str = "bip110-miner";

/// The node software, short form: `Knots 29.4.2`.
pub const NODE_NAME: &str = "Knots";

/// What a block reward is paid in.
pub const UNIT: &str = "coins";

/// What setup says before asking where rewards go, a paragraph each.
pub const ADDRESS_INTRO: &[&str] = &[
    "If your computer ever finds a block, the reward goes to an address you own.",
    "For BIP-110 coins, the safest home is a wallet on this node. A normal Bitcoin wallet app \
     will accept the address, but can't spend the coins.",
];

/// The ways to get a reward address, in the order offered, recommended first.
pub const ADDRESS_WAYS: [(Way, &str); 2] = [
    (Way::NewWallet, "Create a wallet here, protected by a passphrase   (recommended)"),
    (Way::Paste, "I already have a BIP-110 address — let me paste it"),
];

/// What a wallet made by setup is called.
pub const WALLET_NAME: &str = "bip110-rewards";

/// Roughly how much disk the node needs on `network`: the blocks it keeps,
/// the coin database, and room for the quick-start snapshot while it loads.
pub const fn disk_needed(network: Network) -> u64 {
    match network {
        Network::Mainnet => 32_000_000_000,
        Network::Testnet4 => 6_000_000_000,
        Network::Regtest => 1_000_000_000,
    }
}

/// A UTXO snapshot the node can start from.
pub struct Snapshot {
    /// The block it describes.
    pub height: u32,
    /// Where to download it.
    pub url: &'static str,
    /// Its file name.
    pub file: &'static str,
    /// Its size.
    pub bytes: u64,
}

/// The quick-start snapshot for `network`, where there is one.
///
/// Block 910,000 predates the fork, so this is a plain Bitcoin snapshot, and
/// Knots carries its hash. The mirror is a convenience only: the node checks
/// the file against that hash and refuses anything else.
pub const fn snapshot(network: Network) -> Option<Snapshot> {
    match network {
        Network::Mainnet => Some(Snapshot {
            height: 910_000,
            url: "https://files-vps02.jaonoctus.dev/utxo-910000.dat",
            file: "utxo-910000.dat",
            bytes: 9_637_809_744,
        }),
        Network::Testnet4 | Network::Regtest => None,
    }
}

/// Readies a regtest chain to be mined through the pool.
///
/// The pool builds only BLAKE2b work, and the blocks before the fork use the
/// old 80-byte header. So a brand-new chain gets those first blocks from the
/// node's own generator, paid to an anyone-can-spend script since regtest coins
/// are worthless. Returns what was done, if anything.
pub fn prepare_regtest(client: &RpcClient) -> Result<Option<String>, String> {
    let tip = client.get_blockchain_info().map_err(|error| error.to_string())?.blocks;
    let fork = crate::node::regtest_fork_height();
    // The pool can take over once the next block is the fork block.
    let needed = fork.saturating_sub(1).saturating_sub(tip);
    if needed == 0 {
        return Ok(None);
    }
    let info: Value =
        client.call("getdescriptorinfo", json!(["raw(51)"])).map_err(|error| error.to_string())?;
    let descriptor = info.get("descriptor").and_then(Value::as_str).ok_or("the node gave no descriptor")?;
    client
        .call::<Value>("generatetodescriptor", json!([needed, descriptor]))
        .map_err(|error| format!("cannot mine the blocks before the fork: {error}"))?;
    Ok(Some(format!(
        "a new regtest chain: the node mined its first {needed} blocks itself, since they come \
         before the fork and the pool only builds BLAKE2b work"
    )))
}

/// Where Knots 29.4.2's longer coinbase maturity starts on mainnet.
const LONG_MATURITY_MAINNET: u32 = 973_440;

/// Where it is enforced on testnet4.
const LONG_MATURITY_TESTNET4: u32 = 151_550;

/// Blocks a coinbase must wait before it can be spent, for a block at `height`.
///
/// 100 everywhere in Bitcoin. BIP-110's soft fork raised it to 6,480 (about
/// 45 days) from the heights above.
pub fn maturity(network: Network, height: u32) -> u32 {
    let long = match network {
        Network::Mainnet => height >= LONG_MATURITY_MAINNET,
        Network::Testnet4 => height >= LONG_MATURITY_TESTNET4,
        Network::Regtest => false,
    };
    if long { 6_480 } else { 100 }
}

/// The maturity wait in words, for the line under the reward address.
pub fn maturity_words(network: Network, height: u32) -> &'static str {
    if maturity(network, height) == 6_480 { "45 days" } else { "100 blocks" }
}

/// How many of `peers` are on this chain, where that can be told apart.
///
/// BIP-110 shares Bitcoin's network, so a node's peers are a mix. Those on
/// the fork advertise `NODE_BLAKE2B`, service bit 28.
pub fn chain_peers(peers: &[Value]) -> Option<usize> {
    Some(peers.iter().filter(|peer| advertises_blake2b(peer)).count())
}

/// The label for [`chain_peers`]: `12 peers (9 BIP-110)`.
pub const CHAIN_LABEL: &str = "BIP-110";

/// Whether a peer advertises `NODE_BLAKE2B`, the fork's service bit.
pub fn advertises_blake2b(peer: &Value) -> bool {
    peer.get("services")
        .and_then(Value::as_str)
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .is_some_and(|services| services & (1 << 28) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maturity_changes_at_the_soft_fork() {
        assert_eq!(maturity(Network::Mainnet, 973_439), 100);
        assert_eq!(maturity(Network::Mainnet, 973_440), 6_480);
        assert_eq!(maturity(Network::Regtest, 2_000_000), 100);
    }
}
