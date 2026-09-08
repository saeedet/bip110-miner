//! Checking a submitted share, and turning a winning one into a block.
//!
//! The pool never takes a miner's word for anything. It rebuilds the exact
//! header the miner claims to have hashed — from the job it issued and the
//! extranonce, offset, and nonce the miner supplied — and hashes it itself.
//!
//! # Why it hashes the header rather than the job
//!
//! The pool has both. It could rebuild the miner's midstate from the two
//! digests it sent and repeat the miner's own computation, which would be
//! cheaper.
//!
//! It runs the full five stages from the header instead, deliberately. A job is
//! a *projection* of a header — version, timestamp, merkle root and previous
//! block all disappear into `h2` — so checking against the job would only prove
//! the miner used the projection correctly, not that the projection was right.
//! Recomputing from the header makes the two paths independent, and a
//! disagreement between them is caught here rather than by a node rejecting a
//! solved block.
//!
//! In solo mining the miner and the pool operator are the same person, so there
//! is nobody to cheat. Doing it properly anyway is what makes it safe to point
//! other hardware at this pool later: firmware we did not write should be
//! verified rather than trusted.

use bip110_primitives::{BlockHeader, Hash256, hex, pow_hash};
use node_rpc::RpcClient;
use stratum::Share;

use crate::job_builder::ActiveJob;

/// Longest we will sit on a solved block waiting for its timestamp to become
/// legal. Bounded so a mistake in the window arithmetic stalls one share rather
/// than the whole pool.
const MAX_SUBMISSION_HOLD: i64 = 30 * 60;

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// What the pool made of a share.
pub enum Verdict {
    /// The hash met the network target and the node accepted the block.
    BlockAccepted {
        /// The block's hash.
        hash: Hash256,
        /// Its height.
        height: u32,
    },
    /// The block was well-formed but did not become the chain tip.
    ///
    /// The node answers `inconclusive` for a valid block that is a sibling of
    /// the current tip, and `duplicate` for one it already has. Neither is a
    /// fault: they mean the work was real but someone — possibly us, moments
    /// earlier — got there first. Distinguishing them from a genuine rejection
    /// matters, because a genuine rejection is always our bug and these are not.
    BlockStale {
        /// The node's reason.
        reason: String,
        /// The block's hash.
        hash: Hash256,
    },
    /// The hash met the network target but the node refused the block.
    ///
    /// Always a bug on our side: the proof of work was real, so the block was
    /// malformed somewhere. The reason string says where.
    BlockRejected {
        /// The node's reason.
        reason: String,
        /// The hash that would have been the block's.
        hash: Hash256,
    },
    /// A valid share that is not a block. Normal, and the overwhelming majority.
    Share {
        /// The hash produced.
        hash: Hash256,
        /// How many leading zero bits it had — the near-miss measure.
        zero_bits: u32,
    },
}

/// Rebuilds and checks a share.
pub fn check(
    client: &RpcClient,
    active: &ActiveJob,
    share: &Share,
    extranonce1: &[u8],
) -> Result<Verdict, ValidationError> {
    // The full extranonce is the pool's half followed by the miner's.
    let mut extranonce = [0u8; 16];
    if extranonce1.len() + share.extranonce2.len() != extranonce.len() {
        return Err(ValidationError::BadExtranonceLength {
            extranonce1: extranonce1.len(),
            extranonce2: share.extranonce2.len(),
        });
    }
    extranonce[..extranonce1.len()].copy_from_slice(extranonce1);
    extranonce[extranonce1.len()..].copy_from_slice(&share.extranonce2);

    let header = BlockHeader {
        // The miner's own choices, not ours.
        extranonce,
        nonce: share.nonce,
        time_offset: share.time_offset,
        // Stratum V1's `mining.submit` has five parameters and no room for
        // these. A miner reached through this protocol therefore leaves them
        // zero and grinds the other three words, which between the 32-bit
        // nonce, the 32-bit offset, and the 64-bit extranonce2 is 2^128 of
        // search space by another route. Nothing is lost by not having a slot.
        nonce2: 0,
        nonce3: 0,
        ..active.header
    };

    // The full five stages, from the header. Not the miner's shortcut.
    let hash = pow_hash(&header);

    if !active.network_target.is_met_by(&hash) {
        return Ok(Verdict::Share {
            hash,
            zero_bits: hash.leading_zero_bits(),
        });
    }

    // A real block. Serialise and submit it.
    let raw = active.block.serialize(&header);

    // A block built for the minimum-difficulty window may carry a timestamp
    // ahead of the wall clock, and nodes reject anything more than two hours
    // ahead as `time-too-new`. That rejection is temporary — the block is not
    // invalid, only early — so the right response is to wait rather than
    // discard perfectly good work.
    if let Some(window) = active.min_difficulty {
        let wait = window.seconds_to_wait(unix_now()).min(MAX_SUBMISSION_HOLD);
        if wait > 0 {
            println!("  block solved early — holding {wait}s until it can be submitted");
            std::thread::sleep(std::time::Duration::from_secs(wait as u64));
        }
    }

    match client
        .submit_block(&hex::encode(&raw))
        .map_err(|error| ValidationError::Rpc(error.to_string()))?
    {
        None => Ok(Verdict::BlockAccepted {
            hash,
            height: active.height,
        }),
        // These two mean "valid, but not the tip" rather than "malformed".
        Some(reason) if reason == "inconclusive" || reason == "duplicate" => {
            Ok(Verdict::BlockStale { reason, hash })
        }
        Some(reason) => Ok(Verdict::BlockRejected { reason, hash }),
    }
}

/// Why a share could not be checked.
#[derive(Debug)]
pub enum ValidationError {
    /// The two extranonce halves did not add up to the header's 16 bytes.
    BadExtranonceLength {
        /// Length of the pool's half.
        extranonce1: usize,
        /// Length the miner sent.
        extranonce2: usize,
    },
    /// Talking to the node failed.
    Rpc(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadExtranonceLength { extranonce1, extranonce2 } => write!(
                f,
                "extranonce halves are {extranonce1} + {extranonce2} bytes; \
                 the header's extranonce is 16"
            ),
            Self::Rpc(message) => write!(f, "node call failed: {message}"),
        }
    }
}

impl std::error::Error for ValidationError {}
