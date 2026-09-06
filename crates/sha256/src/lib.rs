//! SHA-256, and the BIP340 tagged hashes this chain's proof of work is built on.
//!
//! # Why a BLAKE2b chain needs SHA-256
//!
//! Replacing SHA-256d with BLAKE2b did not remove SHA-256 from the proof of
//! work — it moved it. Three of the five stages are BIP340 tagged hashes, which
//! are SHA-256 underneath. They fold the consensus fields into a single digest
//! *before* any BLAKE2b happens, so the mining hardware never sees a version
//! bit or a timestamp and cannot be stranded by a change to either.
//!
//! See `docs/proof-of-work.md` for the full pipeline.
//!
//! # This implementation is deliberately slow
//!
//! Its sibling Bitcoin project spent a whole phase making SHA-256 fast, because
//! there it *is* the hot loop. Here it is not: the tagged hashes are computed
//! once per extranonce and then amortised across an entire nonce sweep. The
//! inner loop is BLAKE2b.
//!
//! So this is the portable, readable FIPS 180-4 implementation and nothing
//! more. Optimising it would buy nothing and cost clarity.

mod constants;
mod padding;

pub mod reference;

/// Computes SHA-256 of `message`.
pub fn sha256(message: &[u8]) -> [u8; 32] {
    reference::sha256(message)
}

/// Computes a BIP340 tagged hash: `SHA256(SHA256(tag) ‖ SHA256(tag) ‖ data)`.
///
/// # Why the tag is hashed and repeated
///
/// Tagging domain-separates: two protocols hashing the same bytes under
/// different tags can never collide, so a signature from one context cannot be
/// replayed in another.
///
/// The repetition is the part worth understanding. `SHA256(tag)` is 32 bytes,
/// so writing it twice fills exactly one 64-byte SHA-256 block. That means the
/// midstate after the prefix depends only on the tag — an implementation that
/// uses a fixed tag can precompute it once and start every hash from there.
/// The node's own assertions count this as `0x40` bytes of overhead, which is
/// where the `0x40 + n` figures in its source come from.
pub fn tagged_hash(tag: &[u8], data: &[u8]) -> [u8; 32] {
    let tag_hash = sha256(tag);

    let mut input = Vec::with_capacity(64 + data.len());
    input.extend_from_slice(&tag_hash);
    input.extend_from_slice(&tag_hash);
    input.extend_from_slice(data);

    sha256(&input)
}

/// A tagged hash built up in pieces.
///
/// The proof of work feeds a dozen separate fields into each tagged hash, so
/// building the input by hand at every call site would be noisy and easy to get
/// out of order. This accumulates them instead.
pub struct TaggedHasher {
    buffer: Vec<u8>,
}

impl TaggedHasher {
    /// Starts a tagged hash, writing the `SHA256(tag) ‖ SHA256(tag)` prefix.
    pub fn new(tag: &[u8]) -> Self {
        let tag_hash = sha256(tag);

        let mut buffer = Vec::with_capacity(64);
        buffer.extend_from_slice(&tag_hash);
        buffer.extend_from_slice(&tag_hash);

        Self { buffer }
    }

    /// Appends raw bytes.
    pub fn write(&mut self, bytes: &[u8]) -> &mut Self {
        self.buffer.extend_from_slice(bytes);
        self
    }

    /// Appends a `u8`.
    pub fn write_u8(&mut self, value: u8) -> &mut Self {
        self.write(&[value])
    }

    /// Appends a `u32`, little-endian — Bitcoin's serialisation order.
    pub fn write_u32(&mut self, value: u32) -> &mut Self {
        self.write(&value.to_le_bytes())
    }

    /// How many bytes have been written, prefix included.
    ///
    /// The node asserts on this exact figure at each stage (`0x40 + 119`, and
    /// so on), and reproducing those assertions is the cheapest way to know a
    /// field has not been missed or misordered.
    pub fn bytes_written(&self) -> usize {
        self.buffer.len()
    }

    /// Finishes the hash.
    pub fn finalize(&self) -> [u8; 32] {
        sha256(&self.buffer)
    }
}
