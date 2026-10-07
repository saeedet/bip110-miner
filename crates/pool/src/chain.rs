//! The one chain parameter a template does not carry.
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
            std::env::var("BIP110_HEADLINE")
                .unwrap_or_else(|_| "bip110-miner regtest".to_owned())
                .into_bytes(),
        ),
    }
}
