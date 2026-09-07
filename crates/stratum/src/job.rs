//! A unit of work, and how a miner turns it into hashes.
//!
//! `mining.notify` carries the same nine positional parameters it has carried
//! since 2012:
//!
//! ```text
//! [job_id, prevhash, coinb1, coinb2, merkle_branch, version, nbits, ntime, clean_jobs]
//! ```
//!
//! Every one of them still has a job here. Two of them are empty, and those
//! two are the interesting ones.
//!
//! # What each slot carries on this chain
//!
//! | Slot | Bitcoin | Here |
//! |---|---|---|
//! | `prevhash` | the previous block hash | `prevblock_hidden`, with its first 6 bytes blanked |
//! | `coinb1` | coinbase bytes before the extranonce | three zero bytes, then `h2` — the digest of every consensus field |
//! | `coinb2` | coinbase bytes after the extranonce | **empty** |
//! | `merkle_branch` | the path from the coinbase leaf to the root | **empty** |
//! | `version` | the header version to hash | the four stage-4 layout flags |
//! | `nbits` | the block's difficulty target | unchanged |
//! | `ntime` | the header timestamp | `time_offset`, which the miner rolls freely |
//! | `clean_jobs` | discard older jobs | unchanged |
//!
//! # The two empty slots
//!
//! `coinb2` and `merkle_branch` exist for one reason: on Bitcoin the extranonce
//! is spliced into the middle of the coinbase transaction, so a miner needs the
//! bytes on both sides of the splice and the merkle path to recompute the root
//! afterwards. Every extranonce increment costs a partial merkle recomputation.
//!
//! This fork put the extranonce in the header. The coinbase is no longer
//! touched by mining, the merkle root never moves, and both slots have nothing
//! to carry. Sending them empty is not a degenerate case — it is the protocol
//! stating what the hardfork did.
//!
//! # What a miner is *not* told
//!
//! Not the block version, not the timestamp, not the merkle root, not the real
//! previous block hash. All of them are folded into `h2` before the job is
//! built, and the node's source says why:
//!
//! > *"These fields are invisible to the mining machine. This means the hasher
//! > cannot brick itself at some future block version, time, or difficulty."*
//!
//! Nor is a miner told the XOR mask. That one is deliberate in the other
//! direction: a pool publishes the *hash* of its XOR key and keeps the key, so
//! a miner can recognise a good share but not a block, and therefore cannot
//! withhold one. A solo miner is its own pool and uses a null key, which means
//! no mask at all — see [`Job::masked`].
//!
//! # Byte order
//!
//! Bitcoin's Stratum sends `prevhash` with each 4-byte word reversed, a
//! convention inherited from hardware that loaded the header as eight 32-bit
//! words. It is the protocol's most notorious trap.
//!
//! It does not apply here, and this module deliberately does not reproduce it.
//! What travels in the `prevhash` slot is not a hash being displayed; it is 32
//! bytes fed verbatim into BLAKE2b. Any reordering would have to be undone
//! before hashing, so the only sane wire form is the order it is hashed in.

use btcb2_primitives::{PowMidstate, Target, hex};
use serde_json::{Value, json};

/// Bytes of extranonce the pool assigns to a connection.
///
/// The header's extranonce is 16 bytes, split the way Stratum always splits it:
/// a pool-assigned prefix that makes two miners' search spaces disjoint, and a
/// suffix the miner varies itself.
pub const EXTRANONCE1_SIZE: usize = 8;

/// Bytes of extranonce the miner chooses. `16 - EXTRANONCE1_SIZE`.
pub const EXTRANONCE2_SIZE: usize = 8;

/// What the pool assigns a miner when it subscribes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// Per-connection prefix, chosen by the pool. Guarantees two miners never
    /// hash the same input even if they pick the same `extranonce2`.
    pub extranonce1: Vec<u8>,
    /// How many bytes of `extranonce2` the miner is expected to supply.
    pub extranonce2_size: usize,
}

/// One job: everything needed to hash, and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Identifies this job when a share is submitted.
    pub job_id: String,
    /// `prevblock_hidden` with its first 6 bytes zeroed, as stage 4 layout 0
    /// consumes it. Travels in the `prevhash` slot.
    pub prev_hidden: [u8; 32],
    /// `h2` — the merge-mining digest, which transitively commits to every
    /// consensus field in the header. Travels in the `coinb1` slot.
    pub consensus_digest: [u8; 32],
    /// Which of the four stage-4 layouts to use. Travels in the `version` slot.
    pub flags: u8,
    /// The block's compact difficulty target.
    pub bits: u32,
    /// Whether the miner must discard previous jobs.
    ///
    /// Set when a new block arrives. Anything built on the old tip is worthless
    /// the instant the chain moves, so continuing to hash it wastes power.
    pub clean_jobs: bool,
}

impl Job {
    /// Encodes as `mining.notify` parameters.
    pub fn to_notify_params(&self) -> Value {
        json!([
            self.job_id,
            hex::encode(&self.prev_hidden),
            // The leading zero word: the node writes `(uint32_t)0` and calls
            // three of its four bytes part of coinb1, the fourth "implied by
            // the hasher". We send all four and let the miner treat them as
            // one field, because a three-byte prefix with an implied fourth is
            // a hardware detail, not a wire one.
            format!("00000000{}", hex::encode(&self.consensus_digest)),
            "",
            Value::Array(Vec::new()),
            format!("{:08x}", u32::from(self.flags)),
            format!("{:08x}", self.bits),
            "00000000",
            self.clean_jobs,
        ])
    }

    /// Decodes `mining.notify` parameters.
    pub fn from_notify_params(params: &Value) -> Result<Self, JobError> {
        let array = params.as_array().ok_or(JobError::NotAnArray)?;
        if array.len() < 9 {
            return Err(JobError::WrongParamCount(array.len()));
        }

        let text = |index: usize| -> Result<&str, JobError> {
            array[index].as_str().ok_or(JobError::NotAString(index))
        };

        let coinb1 = hex::decode(text(2)?)?;
        if coinb1.len() != 36 {
            return Err(JobError::BadCoinb1Length(coinb1.len()));
        }

        // A non-empty coinb2 or merkle branch means the sender believes the
        // extranonce moves the merkle root — that is, it thinks this is
        // Bitcoin. Better to say so than to mine something nobody will accept.
        if !text(3)?.is_empty() {
            return Err(JobError::UnexpectedCoinbaseSuffix);
        }
        if array[4].as_array().is_some_and(|branch| !branch.is_empty()) {
            return Err(JobError::UnexpectedMerkleBranch);
        }

        Ok(Self {
            job_id: text(0)?.to_owned(),
            prev_hidden: hex::decode_array::<32>(text(1)?)?,
            consensus_digest: coinb1[4..].try_into().expect("checked to be 36 bytes"),
            flags: u32::from_str_radix(text(5)?, 16)
                .map_err(|_| JobError::NotHex(5))?
                .try_into()
                .map_err(|_| JobError::FlagsOutOfRange)?,
            bits: u32::from_str_radix(text(6)?, 16).map_err(|_| JobError::NotHex(6))?,
            clean_jobs: array[8].as_bool().unwrap_or(false),
        })
    }

    /// The difficulty target a solved hash must meet.
    pub fn target(&self) -> Result<Target, JobError> {
        Target::from_compact(self.bits).map_err(JobError::BadTarget)
    }

    /// Builds the midstate for this job and a chosen `extranonce2`.
    ///
    /// This is stage 3, and it is the boundary the whole design is arranged
    /// around: everything above it is fixed for the job, everything below it is
    /// the miner's to grind. Recomputing it costs one BLAKE2b, and a miner only
    /// pays that when it moves its extranonce — which, with 2⁶⁴ nonces below
    /// each one, it never will.
    pub fn midstate(&self, extranonce1: &[u8], extranonce2: &[u8]) -> Result<PowMidstate, JobError> {
        let mut extranonce = [0u8; 16];
        if extranonce1.len() + extranonce2.len() != extranonce.len() {
            return Err(JobError::BadExtranonceLength {
                extranonce1: extranonce1.len(),
                extranonce2: extranonce2.len(),
            });
        }
        extranonce[..extranonce1.len()].copy_from_slice(extranonce1);
        extranonce[extranonce1.len()..].copy_from_slice(extranonce2);

        Ok(PowMidstate::from_stratum_job(
            &self.consensus_digest,
            &extranonce,
            &self.prev_hidden,
            self.flags,
        ))
    }

    /// Whether hashes from this job are masked, and so cannot be judged by the
    /// miner that produced them.
    ///
    /// Always false as this project builds jobs, because a solo miner uses a
    /// null XOR key. Present because the distinction is the entire point of
    /// the mechanism: with a mask in play, a miner's "is this a block?" test is
    /// answerable only by the pool, which is what makes withholding impossible.
    pub const fn masked(&self) -> bool {
        false
    }
}

/// Why a job could not be read off the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// `params` was not a JSON array.
    NotAnArray,
    /// `mining.notify` takes nine parameters.
    WrongParamCount(usize),
    /// A parameter that must be a string was not.
    NotAString(usize),
    /// A parameter that must be hex was not.
    NotHex(usize),
    /// A hex field could not be decoded.
    BadHex(hex::HexError),
    /// `coinb1` must be the zero word followed by the 32-byte digest.
    BadCoinb1Length(usize),
    /// `coinb2` was not empty, which means the sender thinks the extranonce
    /// moves the merkle root.
    UnexpectedCoinbaseSuffix,
    /// `merkle_branch` was not empty, for the same reason.
    UnexpectedMerkleBranch,
    /// The flags field did not fit in a byte.
    FlagsOutOfRange,
    /// The two extranonce halves did not add up to 16 bytes.
    BadExtranonceLength {
        /// Length of the pool's half.
        extranonce1: usize,
        /// Length of the miner's half.
        extranonce2: usize,
    },
    /// The `nbits` field did not decode to a usable target.
    BadTarget(btcb2_primitives::target::TargetError),
}

impl From<hex::HexError> for JobError {
    fn from(error: hex::HexError) -> Self {
        Self::BadHex(error)
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnArray => write!(f, "mining.notify params were not an array"),
            Self::WrongParamCount(n) => write!(f, "mining.notify takes 9 parameters, got {n}"),
            Self::NotAString(i) => write!(f, "mining.notify parameter {i} was not a string"),
            Self::NotHex(i) => write!(f, "mining.notify parameter {i} was not hex"),
            Self::BadHex(source) => write!(f, "bad hex in mining.notify: {source}"),
            Self::BadCoinb1Length(n) => write!(
                f,
                "coinb1 is {n} bytes; expected 36 (a zero word, then the 32-byte digest)"
            ),
            Self::UnexpectedCoinbaseSuffix => write!(
                f,
                "coinb2 was not empty — on this chain the extranonce is in the header, \
                 so the coinbase is not split and there is nothing to put after it"
            ),
            Self::UnexpectedMerkleBranch => write!(
                f,
                "merkle_branch was not empty — on this chain the merkle root does not \
                 move while mining, so there is no path to recompute"
            ),
            Self::FlagsOutOfRange => write!(f, "the layout flags did not fit in a byte"),
            Self::BadExtranonceLength { extranonce1, extranonce2 } => write!(
                f,
                "extranonce halves are {extranonce1} + {extranonce2} bytes; the header's \
                 extranonce is 16"
            ),
            Self::BadTarget(source) => write!(f, "bad nbits in mining.notify: {source}"),
        }
    }
}

impl std::error::Error for JobError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        Job {
            job_id: "7".to_owned(),
            prev_hidden: [0xAB; 32],
            consensus_digest: [0xCD; 32],
            flags: 0,
            bits: 0x207F_FFFF,
            clean_jobs: true,
        }
    }

    #[test]
    fn notify_params_round_trip() {
        let params = job().to_notify_params();
        assert_eq!(Job::from_notify_params(&params).expect("parses"), job());
    }

    /// The two empty slots are load-bearing. A sender that fills them has
    /// mistaken this chain for Bitcoin, and the resulting work would be built
    /// on a merkle root that does not match the block.
    #[test]
    fn a_bitcoin_shaped_job_is_rejected() {
        let mut params = job().to_notify_params();

        params[3] = json!("deadbeef");
        assert_eq!(
            Job::from_notify_params(&params),
            Err(JobError::UnexpectedCoinbaseSuffix)
        );

        let mut params = job().to_notify_params();
        params[4] = json!(["00".repeat(32)]);
        assert_eq!(
            Job::from_notify_params(&params),
            Err(JobError::UnexpectedMerkleBranch)
        );
    }

    /// `coinb1` is the zero word plus the digest, and nothing else fits.
    #[test]
    fn coinb1_length_is_checked() {
        let mut params = job().to_notify_params();
        params[2] = json!("00000000cdcd");

        assert_eq!(
            Job::from_notify_params(&params),
            Err(JobError::BadCoinb1Length(6))
        );
    }

    /// The extranonce halves must add up to the header field they fill.
    #[test]
    fn extranonce_halves_must_total_sixteen() {
        let result = job().midstate(&[0u8; 8], &[0u8; 4]);
        assert_eq!(
            result.err(),
            Some(JobError::BadExtranonceLength { extranonce1: 8, extranonce2: 4 })
        );
    }
}
