//! Typed wrappers for the handful of RPCs a solo miner actually calls.
//!
//! A small surface: where the tip is, what to mine, how to submit the result,
//! and — on regtest — a throwaway address to pay itself.

use crate::client::{RpcClient, RpcError};
use crate::template::BlockTemplate;

use serde::Deserialize;
use serde_json::json;

/// The subset of `getblockheader` this project uses.
///
/// The RPC reports the header version in **two** fields, and the split is
/// deliberate: `version` has the v2 flag masked off, and `header_version` says
/// whether the flag was set — 2 for a 164-byte header, absent for an 80-byte
/// one. Reading only `version` would make a post-fork block indistinguishable
/// from a pre-fork one.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockHeaderInfo {
    /// The block's hash, in display order.
    pub hash: String,
    /// Its height.
    pub height: u32,
    /// Its timestamp.
    pub time: u32,
    /// Its compact difficulty target, as hex.
    pub bits: String,
    /// `Some(2)` for a v2 header; absent for the classic 80-byte one.
    #[serde(default)]
    pub header_version: Option<u32>,
}

impl BlockHeaderInfo {
    /// Whether this block carries a 164-byte v2 header.
    pub fn is_header_v2(&self) -> bool {
        self.header_version.is_some_and(|v| v >= 2)
    }
}

/// The result of validating an address.
#[derive(Debug, Clone, Deserialize)]
pub struct AddressInfo {
    /// Whether the address is well-formed **and** belongs to this network.
    #[serde(rename = "isvalid")]
    pub is_valid: bool,
    /// The locking script that pays to it, as hex. Absent when invalid.
    #[serde(rename = "scriptPubKey")]
    pub script_pubkey: Option<String>,
}

/// The subset of `getblockchaininfo` this project uses.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainInfo {
    /// Network name: `main`, `test`, or `regtest`.
    pub chain: String,
    /// Height of the most-work fully-validated chain.
    pub blocks: u32,
    /// Height of the highest header we know of. Ahead of `blocks` while
    /// blocks are still being downloaded.
    pub headers: u32,
    /// The tip's hash, in display order.
    #[serde(rename = "bestblockhash")]
    pub best_block_hash: String,
    /// Whether the node is still catching up.
    ///
    /// Mining while this is true is pointless: the tip we would build on is not
    /// the network's tip, so any block found would be rejected.
    #[serde(rename = "initialblockdownload")]
    pub initial_block_download: bool,
    /// Whether the node is pruned.
    pub pruned: bool,
    /// Timestamp of the tip block.
    #[serde(default)]
    pub time: u64,
}

impl RpcClient {
    /// Where the chain tip is, and whether the node is caught up.
    pub fn get_blockchain_info(&self) -> Result<BlockchainInfo, RpcError> {
        self.call("getblockchaininfo", json!([]))
    }

    /// Asks the node what to mine.
    ///
    /// # Why two rules and not one
    ///
    /// `segwit` has been required since 2017: the node will not hand a
    /// SegWit-era template to a client that has not said it understands
    /// SegWit, because such a client would build an invalid block.
    ///
    /// `blake2b` is this fork's equivalent and fails the same way, loudly:
    ///
    /// ```text
    /// error code: -8
    /// error message:
    /// Support for 'blake2b' rule requires explicit client support
    /// ```
    ///
    /// That refusal is the point. A miner that did not declare the rule would
    /// otherwise receive a template whose version bit asks for a 164-byte
    /// header, build the 80-byte one it knows about, hash it with SHA-256d,
    /// and grind forever on a block the network would never accept. Declaring
    /// the rule is a promise that we understand the new header — which is what
    /// the rest of this project is.
    ///
    /// Both rules stay declared before the fork height too. Declaring support
    /// for a rule that is not yet active costs nothing; the node simply does
    /// not set the version bit, and [`BlockTemplate::header_v2`] reports false.
    pub fn get_block_template(&self) -> Result<BlockTemplate, RpcError> {
        self.call(
            "getblocktemplate",
            json!([{ "rules": ["segwit", "blake2b"], "mode": "template" }]),
        )
    }

    /// Submits a solved block.
    ///
    /// Returns `None` when the block was accepted. Anything else is the node's
    /// reason for rejecting it — `high-hash`, `bad-cb-height`,
    /// `bad-txnlist-size`, `bad-headline` and so on. This is the single most
    /// important return value in the project, so it is deliberately not
    /// collapsed into a boolean: the reason string is what tells you which
    /// part of block assembly is wrong.
    pub fn submit_block(&self, raw_block_hex: &str) -> Result<Option<String>, RpcError> {
        self.call("submitblock", json!([raw_block_hex]))
    }

    /// Asks a named wallet for a new address.
    ///
    /// Used on regtest to get a throwaway payout address. On the public
    /// networks the address comes from the user instead, from a wallet whose
    /// keys they control.
    ///
    /// The wallet is named rather than left to the node to guess — see
    /// [`RpcClient::call_wallet`] for why that distinction has teeth.
    pub fn get_new_address(&self, wallet: &str) -> Result<String, RpcError> {
        self.call_wallet(wallet, "getnewaddress", json!(["", "bech32"]))
    }

    /// How many peers the node is connected to.
    pub fn get_connection_count(&self) -> Result<u32, RpcError> {
        self.call("getconnectioncount", json!([]))
    }

    /// Fetches a block header.
    ///
    /// Used to look at the block we are building on. The one thing the
    /// template does not say is whether the *parent* was a v2 block, and that
    /// is precisely what distinguishes the fork block — the single height at
    /// which the coinbase must carry the headline — from every block after it.
    pub fn get_block_header(&self, hash: &str) -> Result<BlockHeaderInfo, RpcError> {
        self.call("getblockheader", json!([hash, true]))
    }

    /// Validates an address and returns its `scriptPubKey`.
    ///
    /// Wallet-independent, so it works for an address the node has never seen.
    ///
    /// This is the safety gate for the payout address. A wrong-network or
    /// typo'd address does not fail loudly at mining time: it produces a
    /// perfectly valid block that pays to nothing recoverable. Checking here
    /// means the miner refuses to start instead.
    ///
    /// # A hazard specific to this fork
    ///
    /// This chain kept Bitcoin's address format *and* its network magic, so a
    /// Bitcoin address validates here and a BTCB2 address validates on
    /// Bitcoin. This call proves an address is well-formed for the network;
    /// it cannot prove you meant to mine this chain rather than the other one.
    pub fn validate_address(&self, address: &str) -> Result<AddressInfo, RpcError> {
        self.call("validateaddress", json!([address]))
    }

    /// Creates a wallet, or loads it if it already exists.
    ///
    /// Regtest convenience: a fresh datadir has no wallet, and `getnewaddress`
    /// fails without one. Idempotent, so it is safe to call on every start.
    pub fn ensure_wallet(&self, name: &str) -> Result<(), RpcError> {
        // A wallet that already exists produces an RPC error rather than a
        // success, and "already there" is exactly the state we want, so both
        // outcomes are fine. Any other failure is a real one.
        let created: Result<serde_json::Value, _> =
            self.call("createwallet", json!([name, false, false, "", false, true]));

        match created {
            Ok(_) => Ok(()),
            Err(RpcError::Rpc { code, .. }) if code == -4 || code == -35 => {
                // -4 — wallet already exists on disk; -35 — already loaded.
                let loaded: Result<serde_json::Value, _> = self.call("loadwallet", json!([name]));
                match loaded {
                    Ok(_) => Ok(()),
                    // -35 again means another process already loaded it. Fine.
                    Err(RpcError::Rpc { code: -35, .. }) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }
}
