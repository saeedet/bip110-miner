//! A 256-bit hash that knows which way round it goes.
//!
//! Inherited wholesale from Bitcoin, along with its worst ergonomic wart:
//! hashes are stored in one byte order and **displayed** in the opposite one.
//!
//! ```text
//! displayed:  250c77d7227d48cfb8e74e7ae3d700044fc56a2be3984bf34230434167a48209
//! in memory:  0982a46741433042f34b98e32b6ac54f0400d7e37a4ee7b8cf487d22d7770c25
//! ```
//!
//! This type stores **internal** order — the bytes as they appear in a
//! serialised header — and does the reversal only in `Display` and `FromStr`.
//! Printing therefore gets the convention automatically and computing gets the
//! wire order automatically, and neither requires remembering anything.
//!
//! # Ordering
//!
//! Deliberately no `Ord`. The internal bytes are least-significant-first, so a
//! byte-wise comparison is not the numeric comparison you want — and it would
//! be wrong *silently*, which is the worst way to be wrong. Difficulty checks
//! go through [`Target::is_met_by`](crate::target::Target::is_met_by), which is
//! explicit about treating a hash as a 256-bit number.

use core::fmt;
use core::str::FromStr;

/// A 256-bit hash, stored in internal (serialisation) byte order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hash256([u8; 32]);

impl Hash256 {
    /// The all-zero hash — a null previous block, or an unset field.
    pub const ZERO: Self = Self([0u8; 32]);

    /// Computes `sha256d(data)` — a txid, a wtxid, or a merkle node.
    ///
    /// Named for the algorithm rather than called `hash`, because this crate
    /// has two hash functions in it and a bare `hash` would silently invite
    /// the wrong one. The header's hash is [`pow_hash`](crate::pow::pow_hash),
    /// which is not this and is never interchangeable with it.
    pub fn sha256d(data: &[u8]) -> Self {
        Self(sha256::sha256d(data))
    }

    /// Wraps bytes already in internal order.
    pub const fn from_internal_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Wraps bytes in display order, reversing them.
    pub fn from_display_bytes(mut bytes: [u8; 32]) -> Self {
        bytes.reverse();
        Self(bytes)
    }

    /// The bytes in internal order — what goes into a header.
    pub const fn as_internal_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The bytes in display order.
    ///
    /// The proof of work needs this: its first stage feeds the *reversed*
    /// previous-block hash into a tagged hash, under the name
    /// `prevblock_ordered_sane` — the source's own editorial comment on which
    /// order is the sane one.
    pub fn to_display_bytes(self) -> [u8; 32] {
        let mut bytes = self.0;
        bytes.reverse();
        bytes
    }

    /// Compares two hashes as the 256-bit numbers they represent.
    ///
    /// Named rather than provided through `Ord` on purpose: the internal bytes
    /// are least-significant-first, so a derived byte-wise ordering would be
    /// wrong, and silently so. Requiring an explicit call means nobody sorts
    /// hashes by accident and gets a plausible-looking wrong answer.
    ///
    /// Walks from the most significant byte and stops at the first difference,
    /// so in the mining loop — where hashes differ almost immediately — this
    /// costs one or two comparisons rather than reversing 32 bytes.
    pub fn numeric_cmp(&self, other: &Self) -> core::cmp::Ordering {
        for i in (0..32).rev() {
            match self.0[i].cmp(&other.0[i]) {
                core::cmp::Ordering::Equal => continue,
                ordering => return ordering,
            }
        }
        core::cmp::Ordering::Equal
    }

    /// Whether this hash is numerically smaller than `other`.
    pub fn is_below(&self, other: &Self) -> bool {
        self.numeric_cmp(other) == core::cmp::Ordering::Less
    }

    /// Counts leading zero **bits** as a reader of the displayed hash would.
    ///
    /// The "how close did I get" figure: worth nothing in consensus terms,
    /// since a near miss is a miss, but it is the only feedback solo mining
    /// ever gives.
    pub fn leading_zero_bits(&self) -> u32 {
        let mut count = 0;
        // Display order is reversed, so the visually-leading bytes are last.
        for byte in self.0.iter().rev() {
            count += byte.leading_zeros();
            if *byte != 0 {
                break;
            }
        }
        count
    }
}

/// Prints reversed, in the convention every explorer uses.
impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter().rev() {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Debug prints the same as Display. A raw byte array in `{:?}` output would be
/// actively misleading, since it is the order nobody quotes.
impl fmt::Debug for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl FromStr for Hash256 {
    type Err = ParseHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 {
            return Err(ParseHashError::WrongLength(s.len()));
        }

        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| ParseHashError::NotHex)?;
        }

        Ok(Self::from_display_bytes(bytes))
    }
}

/// Why a hash string could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHashError {
    /// A hash is exactly 64 hex characters; this was a different length.
    WrongLength(usize),
    /// The string contained a non-hex character.
    NotHex,
}

impl fmt::Display for ParseHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength(n) => write!(f, "expected 64 hex characters, got {n}"),
            Self::NotHex => write!(f, "string contains a non-hex character"),
        }
    }
}

impl std::error::Error for ParseHashError {}
