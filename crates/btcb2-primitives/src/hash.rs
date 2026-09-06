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
