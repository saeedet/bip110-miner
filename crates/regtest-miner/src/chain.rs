//! The two chain-specific values a miner cannot derive from a template.
//!
//! Everything else about a block comes from `getblocktemplate`. These do not,
//! because the node has no reason to publish them: they are its own consensus
//! parameters, and it assumes a miner running this fork already knows them.
//!
//! # The headline
//!
//! At exactly the activation height, the coinbase `scriptSig` must contain a
//! chain-specific byte string, or the block is rejected with `bad-headline`.
//! Mainnet's is a newspaper dateline, in the tradition of the genesis block's
//! `The Times 03/Jan/2009 Chancellor on brink of second bailout for banks` —
//! a hash-verifiable claim that the chain was not started earlier than it says.
//!
//! Testnet sets no headline at all. That is not an oversight in this code: the
//! node's check is `std::search` for an empty sequence, which trivially
//! succeeds, so any coinbase satisfies it.

/// The headline the coinbase must carry at the activation height.
///
/// `None` means the network sets none and the rule cannot fail.
pub fn headline(network: node_rpc::Network) -> Option<Vec<u8>> {
    use node_rpc::Network;

    match network {
        // From `kernel/chainparams.cpp`. One byte wrong and the fork block is
        // rejected, so this is transcribed rather than paraphrased.
        Network::Mainnet => Some(b"8-30 NYPost Deride And Conquer".to_vec()),

        // Testnet leaves `Blake2bHeadline` empty.
        Network::Testnet4 => None,

        // Regtest takes it from `-blake2b_headline`, so it is whatever the
        // config file says. The default here matches `config/bip110.regtest.conf`;
        // override it if you change one without the other.
        Network::Regtest => Some(
            std::env::var("BIP110_HEADLINE")
                .unwrap_or_else(|_| "bip110-miner regtest".to_owned())
                .into_bytes(),
        ),
    }
}
