//! The block header — 80 bytes before the fork, 164 after.
//!
//! Bitcoin's header has been 80 bytes since 2009 and every miner ever built
//! assumes it. This fork replaced it with a 164-byte "v2" header, flagged by
//! setting the top bit of the version field.
//!
//! # Why it grew
//!
//! Not decoration. The upstream change lists what the extra 84 bytes buy:
//! 128 bits of ASIC nonce plus 128 bits of machine nonce, an opt-in fix for
//! block withholding, room for merge mining, and a transaction-count
//! commitment that closes CVE-2017-12842.
//!
//! # The field a miner cares about most
//!
//! `extranonce` is **in the header**. On Bitcoin, rolling the extranonce means
//! rebuilding the coinbase, which changes its txid, which changes the merkle
//! root, which means rebuilding the header. Here the merkle root never moves.
//! That is simpler, and it is why this chain's Stratum job looks different.
//!
//! # Time is stored twice over
//!
//! The wire carries `time_on_wire`; the block's actual timestamp is derived
//! from it and `time_offset`, but only when the `UseTimeOffset` flag is set.
//! The proof of work hashes the *wire* value, so [`BlockHeader::time`] is for
//! consensus checks and display, never for hashing.

use crate::hash::Hash256;

/// A pre-fork header: the classic Bitcoin layout.
pub const HEADER_V1_SIZE: usize = 80;

/// A post-fork header.
pub const HEADER_V2_SIZE: usize = 164;

/// Top bit of the serialised version field, marking a v2 header.
///
/// Chosen because no real block ever set it: Bitcoin's version is a signed
/// 32-bit value used for soft-fork signalling in its low bits, so the sign bit
/// was free.
pub const VERSION_HEADER_V2_FLAG: u32 = 0x8000_0000;

/// `m_flags` bit meaning the timestamp is `time_on_wire + time_offset`.
pub const FLAG_USE_TIME_OFFSET: u8 = 4;

/// A block header, either version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BlockHeader {
    /// Whether this is a v2 header. Serialised as the top version bit.
    pub header_v2: bool,
    /// Block version, with the v2 flag masked off.
    pub version: i32,
    /// The block this builds on.
    pub prev_block: Hash256,
    /// Commitment to the block's transactions.
    pub merkle_root: Hash256,
    /// The timestamp as serialised. See the module docs — this is what the
    /// proof of work hashes, not [`Self::time`].
    pub time_on_wire: u32,
    /// Compact difficulty target.
    pub bits: u32,
    /// The classic nonce.
    pub nonce: u32,

    // --- v2 only; all zero on a v1 header --------------------------------
    /// Second nonce word.
    pub nonce2: u32,
    /// Third nonce word. With `nonce` and `nonce2` this gives 96 bits of
    /// directly-grindable space before the extranonce has to move.
    pub nonce3: u32,
    /// 128-bit extranonce, in the header rather than the coinbase.
    pub extranonce: [u8; 16],
    /// Added to `time_on_wire` when [`FLAG_USE_TIME_OFFSET`] is set.
    pub time_offset: u32,
    /// Transaction count, committed in the header — the CVE-2017-12842 fix.
    pub txcount: u16,
    /// Layout and behaviour flags. The low two bits select which of four
    /// arrangements the mining hardware is handed.
    pub flags: u8,
    /// How many leading bits of the XOR mask are cleared.
    pub xor_key_mask_clear_bits: u8,
    /// The block-withholding key. A pool can publish its *hash* while keeping
    /// this secret, so a miner cannot tell a block from an ordinary share.
    pub xor_key: [u8; 16],
    /// Block height, committed in the header.
    pub height: u32,
    /// Merge-mining right-hand side.
    pub mm_rhs: Hash256,
}

impl BlockHeader {
    /// The version as serialised, with the v2 flag folded back in.
    pub fn complete_version(&self) -> u32 {
        let base = (self.version as u32) & !VERSION_HEADER_V2_FLAG;
        if self.header_v2 {
            base | VERSION_HEADER_V2_FLAG
        } else {
            base
        }
    }

    /// The block's actual timestamp.
    ///
    /// Wrapping, because the field is a `u32` of seconds and consensus treats
    /// it as such rather than saturating at 2106.
    pub fn time(&self) -> u32 {
        if self.flags & FLAG_USE_TIME_OFFSET == 0 {
            self.time_on_wire
        } else {
            self.time_on_wire.wrapping_add(self.time_offset)
        }
    }

    /// How many bytes this header serialises to.
    pub fn serialized_size(&self) -> usize {
        if self.header_v2 { HEADER_V2_SIZE } else { HEADER_V1_SIZE }
    }

    /// Serialises the header.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.serialized_size());

        out.extend_from_slice(&self.complete_version().to_le_bytes());
        out.extend_from_slice(self.prev_block.as_internal_bytes());
        out.extend_from_slice(self.merkle_root.as_internal_bytes());
        out.extend_from_slice(&self.time_on_wire.to_le_bytes());
        out.extend_from_slice(&self.bits.to_le_bytes());
        out.extend_from_slice(&self.nonce.to_le_bytes());

        if self.header_v2 {
            out.extend_from_slice(&self.nonce2.to_le_bytes());
            out.extend_from_slice(&self.nonce3.to_le_bytes());
            out.extend_from_slice(&self.extranonce);
            out.extend_from_slice(&self.time_offset.to_le_bytes());
            out.extend_from_slice(&self.txcount.to_le_bytes());
            out.push(self.flags);
            out.push(self.xor_key_mask_clear_bits);
            out.extend_from_slice(&self.xor_key);
            out.extend_from_slice(&self.height.to_le_bytes());
            out.extend_from_slice(self.mm_rhs.as_internal_bytes());
        }

        debug_assert_eq!(out.len(), self.serialized_size());
        out
    }

    /// Parses a header, detecting its version from the flag bit.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, HeaderError> {
        let mut r = Cursor::new(bytes);

        let raw_version = r.u32()?;
        let header_v2 = raw_version & VERSION_HEADER_V2_FLAG != 0;

        // The declared size must match what we were given exactly. A v2 header
        // truncated to 80 bytes would otherwise parse its first six fields
        // happily and then read nonsense.
        let expected = if header_v2 { HEADER_V2_SIZE } else { HEADER_V1_SIZE };
        if bytes.len() != expected {
            return Err(HeaderError::WrongLength {
                expected,
                found: bytes.len(),
            });
        }

        let mut header = Self {
            header_v2,
            version: (raw_version & !VERSION_HEADER_V2_FLAG) as i32,
            prev_block: r.hash()?,
            merkle_root: r.hash()?,
            time_on_wire: r.u32()?,
            bits: r.u32()?,
            nonce: r.u32()?,
            nonce2: 0,
            nonce3: 0,
            extranonce: [0u8; 16],
            time_offset: 0,
            txcount: 0,
            flags: 0,
            xor_key_mask_clear_bits: 0,
            xor_key: [0u8; 16],
            height: 0,
            mm_rhs: Hash256::ZERO,
        };

        if header_v2 {
            header.nonce2 = r.u32()?;
            header.nonce3 = r.u32()?;
            header.extranonce = r.array::<16>()?;
            header.time_offset = r.u32()?;
            header.txcount = r.u16()?;
            header.flags = r.u8()?;
            header.xor_key_mask_clear_bits = r.u8()?;
            header.xor_key = r.array::<16>()?;
            header.height = r.u32()?;
            header.mm_rhs = r.hash()?;
        }

        Ok(header)
    }
}

/// A bounds-checked cursor. Small enough to live here rather than earn a module.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], HeaderError> {
        let end = self.at + N;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(HeaderError::UnexpectedEnd)?;
        self.at = end;
        Ok(slice.try_into().expect("slice length checked by get()"))
    }

    fn u8(&mut self) -> Result<u8, HeaderError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, HeaderError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, HeaderError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn hash(&mut self) -> Result<Hash256, HeaderError> {
        Ok(Hash256::from_internal_bytes(self.array()?))
    }
}

/// Why a header could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// The input ended before a field did.
    UnexpectedEnd,
    /// The length did not match what the version flag declared.
    WrongLength {
        /// What the flag implied.
        expected: usize,
        /// What was supplied.
        found: usize,
    },
}

impl std::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEnd => write!(f, "input ended before the header did"),
            Self::WrongLength { expected, found } => write!(
                f,
                "the version flag declares a {expected}-byte header but {found} bytes were given"
            ),
        }
    }
}

impl std::error::Error for HeaderError {}
