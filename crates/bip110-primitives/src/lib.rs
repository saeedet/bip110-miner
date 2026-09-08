//! The block header and proof of work of the BLAKE2b Bitcoin fork.
//!
//! Everything here is pure: no I/O, no network, no clock. Given the same bytes
//! it always produces the same bytes, which is what lets it be tested against
//! real blocks pulled off a running chain — and that is how it *is* tested.
//!
//! Start with [`pow`] if you want to understand the chain, and [`header`] if
//! you want to understand its shape.

pub mod hash;
pub mod header;
pub mod hex;
pub mod merkle;
pub mod pow;
pub mod reader;
pub mod target;
pub mod transaction;
pub mod varint;

pub use hash::Hash256;
pub use header::BlockHeader;
pub use pow::{Blake2bMidstate, PowMidstate, pow_hash};
pub use target::Target;
pub use transaction::{OutPoint, Transaction, TxIn, TxOut};
