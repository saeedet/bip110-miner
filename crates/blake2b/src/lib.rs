//! BLAKE2b — the hash function this chain's proof of work is built on.
//!
//! Bitcoin uses double SHA-256. This fork replaced that with a **single,
//! unkeyed BLAKE2b** producing a 32-byte digest, computed over the serialised
//! block header. In the node's own source that is one call:
//!
//! ```text
//! blake2b_nokey(hash.begin(), hash.size(), ss.data(), ss.size())
//! ```
//!
//! No key, no salt, no personalisation, and applied once rather than twice.
//!
//! # Why a fork would do this
//!
//! SHA-256 has been implemented in dedicated silicon for over a decade, so
//! mining it with a general-purpose CPU is hopeless by a factor of millions.
//! BLAKE2b has no such hardware, and is designed to be fast in *software* on
//! 64-bit machines — which puts CPUs and GPUs back within reach of each other.
//! Whether that is a good idea for a currency is a separate argument; it is
//! certainly what makes this chain interesting to point a laptop at.
//!
//! # What will live here
//!
//! The same two-implementation discipline used for SHA-256 elsewhere:
//!
//! 1. A **portable reference** version following RFC 7693 step by step, slow
//!    enough to be checkable against the spec by eye.
//! 2. An **optimised** version, held to the reference by a property test that
//!    asserts the two agree on arbitrary input.
//!
//! The reference is the definition of correctness; the fast one is what runs.
//!
//! Status: scaffolding only. Implemented in Phase 1.
