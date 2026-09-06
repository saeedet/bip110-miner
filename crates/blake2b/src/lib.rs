//! BLAKE2b — one of the two hash functions this chain's proof of work uses.
//!
//! # It is not a drop-in replacement for SHA-256d
//!
//! The obvious reading of "a fork that replaced SHA-256d with BLAKE2b" is that
//! you swap one hash for another and carry on. That is not what happened. The
//! proof of work is a five-stage pipeline that uses **both** functions: three
//! SHA-256 tagged hashes fold in the consensus fields, then two BLAKE2b passes
//! produce the value compared against the target, then an XOR mask and a byte
//! reversal finish it. See `docs/proof-of-work.md`.
//!
//! BLAKE2b's job is the last two stages — including the one the mining hardware
//! actually grinds, which is a single BLAKE2b over roughly a hundred bytes.
//! That inner loop is what this crate has to make fast.
//!
//! # Why a fork would choose it
//!
//! SHA-256 has been implemented in dedicated silicon for over a decade, so
//! mining it with a general-purpose CPU loses by a factor of millions. BLAKE2b
//! has no such hardware and is designed to be fast in *software* on 64-bit
//! machines, which puts CPUs back within reach. Whether that is good for a
//! currency is a separate argument; it is what makes this chain worth pointing
//! a laptop at.
//!
//! # What will live here
//!
//! The same two-implementation discipline the sibling Bitcoin project used for
//! SHA-256:
//!
//! 1. A **portable reference** version following RFC 7693 step by step, slow
//!    enough to be checked against the spec by eye.
//! 2. An **optimised** version, held to the reference by a property test
//!    asserting the two agree on arbitrary input.
//!
//! The reference is the definition of correctness; the fast one is what runs.
//!
//! # This crate
//!
//! [`blake2b_256`] is what the chain uses. [`blake2b_512`] exists because
//! RFC 7693's own worked example is a 512-bit digest, and being able to
//! reproduce it is worth more than the function itself.

mod constants;

pub mod reference;

/// Computes a 256-bit BLAKE2b digest — the size this chain's proof of work uses.
///
/// Note this is **not** a truncated 512-bit digest. The output length is mixed
/// into the initial state, so the two produce entirely unrelated results.
pub fn blake2b_256(message: &[u8]) -> [u8; 32] {
    reference::hash(message, 32)
        .try_into()
        .expect("hash returns exactly the requested length")
}

/// Computes a 512-bit BLAKE2b digest — BLAKE2b's natural output size.
pub fn blake2b_512(message: &[u8]) -> [u8; 64] {
    reference::hash(message, 64)
        .try_into()
        .expect("hash returns exactly the requested length")
}
