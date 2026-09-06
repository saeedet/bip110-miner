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
//! Status: scaffolding only. Implemented in Phase 1.
