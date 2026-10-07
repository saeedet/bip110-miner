//! What this program's chain does differently from its sibling's.
//!
//! The dashboard, the commands and the layout are shared with btc-miner. The
//! handful of facts that are not — the node's name, how long a reward takes
//! to mature, which peers count as being on the right network — live here,
//! so the two projects differ in this file and not all over.

use node_rpc::Network;
use serde_json::Value;

/// The program's name, as typed and as shown.
pub const PROGRAM: &str = "bip110-miner";

/// The node software, short form: `Knots 29.4.2`.
pub const NODE_NAME: &str = "Knots";

/// What a block reward is paid in.
pub const UNIT: &str = "coins";

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
