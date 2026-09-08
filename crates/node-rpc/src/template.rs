//! `getblocktemplate` — how a miner asks the node what to mine.
//!
//! The node does all the work a miner would otherwise have to: it picks which
//! transactions to include, orders them so dependencies come first, checks they
//! fit the size and sigop limits, and works out what the coinbase may pay. What
//! comes back is everything needed to build a block except the coinbase
//! transaction and the nonce.
//!
//! Defined in BIP 22, extended for SegWit by BIP 145.
//!
//! # The fork is almost invisible here
//!
//! A template for a v2 block looks exactly like a template for a v1 block. The
//! new header fields — three nonce words, the 128-bit extranonce, the XOR key,
//! the merge-mining slot — are all chosen by the miner, so the node has nothing
//! to say about them and adds no fields for them.
//!
//! Two things do carry the signal, and this module reads both:
//!
//! * `version` arrives with the top bit set (`0xA0000000` rather than
//!   `0x20000000`), which *is* the v2 flag — see [`BlockTemplate::header_v2`].
//! * `rules` contains `!blake2b`. The `!` prefix means "you must understand
//!   this"; a client that does not is expected to refuse the template.
//!
//! # What the miner still has to do
//!
//! 1. Build the coinbase transaction, which creates the subsidy and collects
//!    the fees.
//! 2. Compute the merkle root over the coinbase and the given transactions.
//! 3. Fill in the v2 header fields consensus dictates — `height` and `txcount`.
//! 4. Search for a nonce that makes the header's proof of work meet the target.

use bip110_primitives::header::VERSION_HEADER_V2_FLAG;
use bip110_primitives::hex::{self, HexError};
use bip110_primitives::{Hash256, Target};
use serde::Deserialize;
use std::str::FromStr;

/// A block template from `getblocktemplate`.
///
/// Only the fields this miner uses are declared. The node sends a good deal
/// more, and unknown fields are ignored rather than rejected, so a node upgrade
/// that adds a field will not break us.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockTemplate {
    /// Block version to put in the header, **including** the v2 flag bit.
    ///
    /// Deserialised as unsigned. Bitcoin's version field is nominally `i32`,
    /// but this fork sets the top bit, and reading that as a sign bit would
    /// turn every post-fork template into a negative number for no reason.
    pub version: u32,

    /// The block we are building on top of, in display order.
    #[serde(rename = "previousblockhash")]
    pub previous_block_hash: String,

    /// Transactions to include, already ordered and validated by the node.
    pub transactions: Vec<TemplateTransaction>,

    /// The most the coinbase may pay out: subsidy plus the fees of every
    /// included transaction. Paying more makes the block invalid; paying less
    /// is legal but donates the difference to nobody.
    #[serde(rename = "coinbasevalue")]
    pub coinbase_value: u64,

    /// The compact difficulty target for the header, as hex.
    pub bits: String,

    /// The height this block will have.
    ///
    /// Needed three times over: BIP34 puts it in the coinbase, the v2 header
    /// commits to it directly, and it is what decides whether this is the fork
    /// block that must carry the headline.
    pub height: u32,

    /// Suggested timestamp.
    #[serde(rename = "curtime")]
    pub current_time: u64,

    /// The earliest timestamp consensus will accept, one second past the median
    /// of the last eleven blocks.
    #[serde(rename = "mintime")]
    pub min_time: u64,

    /// The SegWit commitment output's `scriptPubKey`, as hex.
    ///
    /// Present whenever SegWit is active. We compute this ourselves and assert
    /// the two agree — deriving it is the point, and having the node's answer
    /// to check against makes that safe.
    #[serde(rename = "default_witness_commitment")]
    pub default_witness_commitment: Option<String>,

    /// The consensus rules in force, `!`-prefixed where they are mandatory.
    ///
    /// A post-fork template contains `!blake2b`.
    #[serde(default)]
    pub rules: Vec<String>,

    /// Which parts of the template we are allowed to change.
    #[serde(default)]
    pub mutable: Vec<String>,
}

impl BlockTemplate {
    /// Whether this template calls for a 164-byte v2 header.
    ///
    /// Read from the version bit rather than from `rules`, because the version
    /// bit is the thing that actually gets serialised: if the two ever
    /// disagreed, the bit is what the node would validate against.
    pub fn header_v2(&self) -> bool {
        self.version & VERSION_HEADER_V2_FLAG != 0
    }

    /// The version with the v2 flag masked off, as it is stored in a header.
    pub fn base_version(&self) -> i32 {
        (self.version & !VERSION_HEADER_V2_FLAG) as i32
    }

    /// The previous block hash, parsed.
    pub fn previous_block(&self) -> Result<Hash256, TemplateError> {
        Hash256::from_str(&self.previous_block_hash)
            .map_err(|_| TemplateError::BadHash(self.previous_block_hash.clone()))
    }

    /// The compact target, parsed from the hex `bits` field.
    ///
    /// Note this is a plain big-endian hex number, *not* a byte-reversed hash,
    /// so it does not go through [`Hash256`].
    pub fn compact_bits(&self) -> Result<u32, TemplateError> {
        let bytes = hex::decode_array::<4>(&self.bits)?;
        Ok(u32::from_be_bytes(bytes))
    }

    /// The difficulty target a solved header must meet.
    ///
    /// Nothing here compensates for the fork's one-off target shift. That shift
    /// is applied by the node when it computes `bits` for the first BLAKE2b
    /// block, so by the time it reaches us it is already baked into this field
    /// — a miner that adjusted it again would be mining the wrong difficulty.
    pub fn target(&self) -> Result<Target, TemplateError> {
        Target::from_compact(self.compact_bits()?).map_err(TemplateError::BadTarget)
    }

    /// Total weight of the included transactions, excluding the coinbase.
    pub fn transactions_weight(&self) -> u64 {
        self.transactions.iter().map(|tx| tx.weight).sum()
    }
}

/// One transaction the node wants included.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateTransaction {
    /// The full serialised transaction, as hex, witness included.
    pub data: String,

    /// The transaction id, in display order.
    pub txid: String,

    /// The **witness** transaction id, in display order.
    ///
    /// Confusingly this field is named `hash`, not `wtxid`. For a transaction
    /// with no witness it equals `txid`; otherwise it differs, and it is the
    /// one that goes into the witness commitment.
    pub hash: String,

    /// Fee paid, in satoshis. Already counted in `coinbase_value`.
    #[serde(default)]
    pub fee: i64,

    /// Weight units, for the 4,000,000 block limit.
    #[serde(default)]
    pub weight: u64,
}

impl TemplateTransaction {
    /// The raw transaction bytes.
    pub fn raw(&self) -> Result<Vec<u8>, TemplateError> {
        Ok(hex::decode(&self.data)?)
    }

    /// The transaction id, parsed. Goes into the block's merkle tree.
    pub fn txid(&self) -> Result<Hash256, TemplateError> {
        Hash256::from_str(&self.txid).map_err(|_| TemplateError::BadHash(self.txid.clone()))
    }

    /// The witness transaction id, parsed. Goes into the witness commitment.
    pub fn wtxid(&self) -> Result<Hash256, TemplateError> {
        Hash256::from_str(&self.hash).map_err(|_| TemplateError::BadHash(self.hash.clone()))
    }
}

/// Why a template could not be interpreted.
#[derive(Debug)]
pub enum TemplateError {
    /// A hash field was not 64 hex characters.
    BadHash(String),
    /// A hex field could not be decoded.
    BadHex(HexError),
    /// The `bits` field did not decode to a usable target.
    BadTarget(bip110_primitives::target::TargetError),
}

impl From<HexError> for TemplateError {
    fn from(error: HexError) -> Self {
        Self::BadHex(error)
    }
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadHash(value) => write!(f, "not a valid hash: {value:?}"),
            Self::BadHex(source) => write!(f, "bad hex in template: {source}"),
            Self::BadTarget(source) => write!(f, "bad target in template: {source}"),
        }
    }
}

impl std::error::Error for TemplateError {}
