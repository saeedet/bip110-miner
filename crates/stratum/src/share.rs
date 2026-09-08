//! A submitted share: `mining.submit`.
//!
//! ```text
//! ["worker_name", job_id, extranonce2, ntime, nonce]
//! ```
//!
//! Five fields, and between them they name every degree of freedom a miner has.
//! The pool already knows the job, so it only needs the parts the miner chose.
//! From those it rebuilds the exact input the miner hashed and checks the
//! result for itself.
//!
//! That last point is the security model. The pool never trusts a miner's claim
//! to have found something — it reconstructs the work and verifies it. In solo
//! mining that matters less, since the miner and the operator are the same
//! person, but building it correctly now is what lets other hardware be pointed
//! at this pool later without any of it being taken on faith.
//!
//! # `ntime` is not a time
//!
//! On Bitcoin this slot is the header timestamp, and a miner may roll it only
//! within the window consensus allows — a few seconds of extra search space,
//! bounded by the median of the last eleven blocks and the two-hour future
//! limit.
//!
//! Here it carries `time_offset`, and the whole 32 bits are free. That field is
//! added to the wire timestamp to give the block's real time, but only when the
//! `UseTimeOffset` flag is set; with the flag clear the node reads the
//! timestamp straight off the wire and never looks at the offset, while stage 4
//! of the proof of work hashes it regardless. A pool that leaves the flag clear
//! — as this one does — hands its miners a second full nonce word in a slot
//! that already existed.
//!
//! The pool must still check it. A miner rolling `ntime` past what the pool
//! expects is harmless here, but a pool that *did* set the flag would be
//! accepting shares that change the block's timestamp, so the check belongs in
//! the code rather than in a comment.

use bip110_primitives::hex;
use serde_json::{Value, json};

/// A share submitted by a miner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// The worker name from `mining.authorize`.
    pub worker: String,
    /// Which job this share is for.
    pub job_id: String,
    /// The miner's half of the header extranonce.
    pub extranonce2: Vec<u8>,
    /// The `time_offset` the miner used. See the module docs.
    pub time_offset: u32,
    /// The nonce that produced the winning hash.
    pub nonce: u32,
}

impl Share {
    /// Encodes as `mining.submit` parameters.
    pub fn to_submit_params(&self) -> Value {
        json!([
            self.worker,
            self.job_id,
            hex::encode(&self.extranonce2),
            format!("{:08x}", self.time_offset),
            format!("{:08x}", self.nonce),
        ])
    }

    /// Decodes `mining.submit` parameters.
    pub fn from_submit_params(params: &Value) -> Result<Self, ShareError> {
        let array = params.as_array().ok_or(ShareError::NotAnArray)?;
        if array.len() < 5 {
            return Err(ShareError::WrongParamCount(array.len()));
        }

        let text = |index: usize| -> Result<&str, ShareError> {
            array[index].as_str().ok_or(ShareError::NotAString(index))
        };

        Ok(Self {
            worker: text(0)?.to_owned(),
            job_id: text(1)?.to_owned(),
            extranonce2: hex::decode(text(2)?)?,
            time_offset: u32::from_str_radix(text(3)?, 16).map_err(|_| ShareError::NotHex(3))?,
            nonce: u32::from_str_radix(text(4)?, 16).map_err(|_| ShareError::NotHex(4))?,
        })
    }
}

/// Why a share could not be read off the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    /// `params` was not a JSON array.
    NotAnArray,
    /// `mining.submit` takes five parameters.
    WrongParamCount(usize),
    /// A parameter that must be a string was not.
    NotAString(usize),
    /// A parameter that must be hex was not.
    NotHex(usize),
    /// A hex field could not be decoded.
    BadHex(hex::HexError),
}

impl From<hex::HexError> for ShareError {
    fn from(error: hex::HexError) -> Self {
        Self::BadHex(error)
    }
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnArray => write!(f, "mining.submit params were not an array"),
            Self::WrongParamCount(n) => write!(f, "mining.submit takes 5 parameters, got {n}"),
            Self::NotAString(i) => write!(f, "mining.submit parameter {i} was not a string"),
            Self::NotHex(i) => write!(f, "mining.submit parameter {i} was not hex"),
            Self::BadHex(source) => write!(f, "bad hex in mining.submit: {source}"),
        }
    }
}

impl std::error::Error for ShareError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_params_round_trip() {
        let share = Share {
            worker: "mac.0".to_owned(),
            job_id: "3".to_owned(),
            extranonce2: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            time_offset: 0xDEAD_BEEF,
            nonce: 0x0000_002A,
        };

        let params = share.to_submit_params();
        assert_eq!(Share::from_submit_params(&params).expect("parses"), share);
    }

    /// Both numeric fields are 8-character big-endian hex, which is what every
    /// real Stratum client emits. Zero-padding is not cosmetic: a client that
    /// trims leading zeros produces a string other implementations misparse.
    #[test]
    fn numeric_fields_are_padded_hex() {
        let share = Share {
            worker: "w".to_owned(),
            job_id: "1".to_owned(),
            extranonce2: Vec::new(),
            time_offset: 1,
            nonce: 42,
        };

        let params = share.to_submit_params();
        assert_eq!(params[3], json!("00000001"));
        assert_eq!(params[4], json!("0000002a"));
    }
}
