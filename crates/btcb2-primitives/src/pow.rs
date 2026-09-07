//! The five-stage proof of work.
//!
//! Transcribed from `src/primitives/block.cpp` in Bitcoin Knots, and verified
//! against real blocks. See `docs/proof-of-work.md` for the full annotated
//! version; the short story is that "BLAKE2b instead of SHA-256d" is wrong on
//! three counts — it uses both functions, applies BLAKE2b twice, and hashes a
//! chain of digests rather than the header.
//!
//! # The split that matters
//!
//! The staging is not incidental. Two comments in the node's source explain it:
//!
//! > *"These fields are invisible to the mining machine. This means the hasher
//! > cannot brick itself at some future block version, time, or difficulty."*
//!
//! > *"Presumably the actual mining ASIC hardware sees these"*
//!
//! Bitcoin ASICs hash the raw 80-byte header, so they bake in assumptions about
//! its layout — which is how a version-bit change can strand hardware. Here
//! everything consensus-shaped is folded into one digest by the host first, and
//! the hardware is handed only that digest plus some nonces. It cannot go stale
//! because it never learns what it is hashing.
//!
//! For a miner this is a gift. Where Bitcoin's midstate optimisation had to be
//! *discovered* — noticing that the first 64 bytes of the header never change —
//! here the boundary is stated by the design. [`PowMidstate`] is exactly the
//! part that survives a nonce sweep.

use crate::hash::Hash256;
use crate::header::BlockHeader;

/// Domain-separation tags, spelled exactly as the node spells them. A single
/// character wrong here produces a hash that is wrong and looks fine.
mod tags {
    /// Commits to the XOR key without revealing it.
    pub const XOR_KEY: &[u8] = b"Bitcoin block hash PoW XOR key";
    /// Derives the mask applied to the final digest.
    pub const XOR_MASK: &[u8] = b"Bitcoin block hash PoW XOR mask";
    /// Hides the previous block hash from the mining hardware.
    pub const PREVBLOCK: &[u8] = b"Bitcoin prevblock header, hashed";
    /// Folds in every consensus field.
    pub const HEADER_1: &[u8] = b"Bitcoin block header 1";
    /// Reserves a place for merge-mined sidechains.
    pub const MERGE_MINING: &[u8] = b"Merge-mining hook";
}

/// The part of the proof of work that does not change while grinding nonces.
///
/// # Two algorithms, one type
///
/// A chain that hardforks its proof of work has, by definition, two of them,
/// and both are live: every node still validates the pre-fork history, and a
/// regtest chain mined from genesis crosses the boundary on the way up. So
/// this is an enum rather than the BLAKE2b pipeline alone.
///
/// ```text
///     if (!m_header_v2) {  // SHA256d
///         // Historical algorithm and common case.
///         Assume(AreHeaderV2FieldsNull());
///         return (HashWriter{} << *this).GetHash();
///     }
/// ```
///
/// That `Assume` is the part with teeth for a miner: on a v1 header the three
/// extra nonce words and the time offset are not merely ignored, they are not
/// serialised at all. Grinding them would compute the same hash forever. See
/// [`Self::nonce_words`], which is how the search learns not to.
#[derive(Clone)]
pub enum PowMidstate {
    /// Pre-fork: plain SHA-256d over the 80-byte header.
    ///
    /// No midstate optimisation. The sibling Bitcoin project has one, because
    /// there this is the hot loop; here it runs only on the handful of regtest
    /// blocks below the activation height, and a fast path would be code that
    /// exists to be admired rather than used.
    Sha256d {
        /// The serialised header. Only bytes 76..80, the nonce, ever change.
        header: [u8; crate::header::HEADER_V1_SIZE],
    },
    /// Post-fork: stages 0 to 3 of the five-stage pipeline.
    ///
    /// The XOR mask, the consensus digest, the merge-mining hook, and the
    /// first BLAKE2b. All depend on the template and the extranonce, so they
    /// are computed once and reused across an entire sweep.
    Blake2b(Blake2bMidstate),
}

/// The precomputed half of the BLAKE2b proof of work.
#[derive(Clone)]
pub struct Blake2bMidstate {
    /// Output of the first BLAKE2b — the value the hardware is handed.
    stage3: [u8; 32],
    /// The merge-mining digest, needed again by two of the four layouts.
    h2: [u8; 32],
    /// The previous block hash, hidden behind a tagged hash.
    prev_hidden: [u8; 32],
    /// Mask XORed into the final digest.
    xor_mask: [u8; 32],
    /// Selects which of four arrangements stage 4 uses.
    flags: u8,
}

impl PowMidstate {
    /// Precomputes whatever `header`'s version allows.
    pub fn new(header: &BlockHeader) -> Self {
        if header.header_v2 {
            Self::Blake2b(Blake2bMidstate::new(header))
        } else {
            let serialized = header.serialize();
            Self::Sha256d {
                header: serialized
                    .try_into()
                    .expect("a v1 header serialises to exactly HEADER_V1_SIZE bytes"),
            }
        }
    }

    /// How many 32-bit words of nonce space this header exposes.
    ///
    /// One before the fork, four after — `nonce`, `nonce2`, `nonce3`, and the
    /// time offset. A search that ignored this on a pre-fork header would roll
    /// words that are never serialised, recomputing one hash forever.
    pub const fn nonce_words(&self) -> u32 {
        match self {
            Self::Sha256d { .. } => 1,
            Self::Blake2b(_) => 4,
        }
    }

    /// Builds a midstate from what a Stratum job carries.
    ///
    /// A pool runs stages 0 to 2 and sends the results; a miner picks up at
    /// stage 3. That split is not this project's invention — it is the same
    /// boundary [`Self::new`] computes, seen from the side that does not have
    /// the header.
    ///
    /// # What is missing, and why that is the point
    ///
    /// There is no XOR mask here, because a miner is never told one. A pool
    /// publishes the *hash* of its XOR key and keeps the key itself, so a
    /// miner can see that a share is good but not that it is a block — which
    /// is what makes block withholding impossible rather than merely
    /// detectable. The mask is applied by the pool, afterwards, in stage 5.
    ///
    /// A solo miner is its own pool and uses a null key, so no mask exists and
    /// the hashes this produces are already final.
    pub fn from_stratum_job(
        consensus_digest: &[u8; 32],
        extranonce: &[u8; 16],
        prev_hidden: &[u8; 32],
        flags: u8,
    ) -> Self {
        Self::Blake2b(Blake2bMidstate {
            stage3: Blake2bMidstate::stage3(consensus_digest, extranonce),
            h2: *consensus_digest,
            prev_hidden: *prev_hidden,
            xor_mask: [0u8; 32],
            flags,
        })
    }

    /// Runs the part of the proof of work that a miner repeats.
    pub fn hash(&self, nonce: u32, nonce2: u32, nonce3: u32, time_offset: u32) -> Hash256 {
        match self {
            Self::Sha256d { header } => {
                // The nonce is the header's last four bytes.
                let mut bytes = *header;
                bytes[76..80].copy_from_slice(&nonce.to_le_bytes());
                Hash256::sha256d(&bytes)
            }
            Self::Blake2b(midstate) => midstate.hash(nonce, nonce2, nonce3, time_offset),
        }
    }
}

impl Blake2bMidstate {
    /// Runs stages 0 to 3 for `header`.
    ///
    /// Only the nonce fields and `time_offset` are ignored — everything else in
    /// the header is consumed here.
    pub fn new(header: &BlockHeader) -> Self {
        // --- stage 0: the XOR key material ---------------------------------
        //
        // A pool can publish `xor_key_hash` while keeping the key itself
        // secret, so a miner cannot tell whether a share it found is actually a
        // block. That is the block-withholding fix: you cannot discard what you
        // cannot recognise.
        let xor_key_hash = sha256::tagged_hash(tags::XOR_KEY, &header.xor_key);

        let xor_mask = if header.xor_key == [0u8; 16] {
            // A null key means no masking at all, not a mask derived from zero.
            [0u8; 32]
        } else {
            let mut mask = sha256::tagged_hash(tags::XOR_MASK, &header.xor_key);

            // Clear the top `xor_key_mask_clear_bits` bits. This is how a pool
            // reveals *some* of the difficulty: a miner can still see that a
            // share is good, just not that it is a block.
            let whole_bytes = usize::from(header.xor_key_mask_clear_bits / 8);
            mask[..whole_bytes].fill(0);
            if let Some(partial) = mask.get_mut(whole_bytes) {
                *partial &= 0xFFu8 >> (header.xor_key_mask_clear_bits % 8);
            }
            mask
        };

        // The source calls this `prevblock_ordered_sane`, which is its own
        // editorial verdict on Bitcoin's byte order.
        let prev_sane = header.prev_block.to_display_bytes();
        let prev_hidden = sha256::tagged_hash(tags::PREVBLOCK, &prev_sane);

        // --- stage 1: every consensus field, in one digest -----------------
        let mut h1 = sha256::TaggedHasher::new(tags::HEADER_1);
        h1.write_u32(header.complete_version())
            .write(&prev_sane)
            .write_u32(header.height)
            .write(header.merkle_root.as_internal_bytes())
            .write_u32(header.time_on_wire)
            .write_u8(0) // reserved for a future 40-bit timestamp
            .write_u32(header.bits)
            .write_u32(u32::from(header.txcount))
            .write_u8(header.flags)
            .write_u8(header.xor_key_mask_clear_bits)
            .write(&xor_key_hash);

        // The node asserts this exact figure. Reproducing the assertion is the
        // cheapest way to know no field has been dropped or reordered.
        debug_assert_eq!(h1.bytes_written(), 0x40 + 119);

        // --- stage 2: the merge-mining hook --------------------------------
        let mut h2_hasher = sha256::TaggedHasher::new(tags::MERGE_MINING);
        h2_hasher
            .write(&h1.finalize())
            .write(&[0u8; 16])
            .write(&[0u8; 16])
            .write(header.mm_rhs.as_internal_bytes());
        debug_assert_eq!(h2_hasher.bytes_written(), 0x40 + 0x60);
        let h2 = h2_hasher.finalize();

        Self {
            stage3: Self::stage3(&h2, &header.extranonce),
            h2,
            prev_hidden,
            xor_mask,
            flags: header.flags,
        }
    }

    /// The consensus digest — `h2` — that a Stratum job carries as `coinb1`.
    pub const fn consensus_digest(&self) -> &[u8; 32] {
        &self.h2
    }

    /// The hidden previous-block hash a Stratum job carries as `prevhash`.
    ///
    /// Returned with its first 6 bytes already blanked, as stage 4 layout 0
    /// consumes it. Blanking is idempotent, so a job that arrives blanked and
    /// is blanked again hashes the same either way.
    pub fn hidden_prev_block(&self) -> [u8; 32] {
        let mut hidden = self.prev_hidden;
        hidden[..6].fill(0);
        hidden
    }

    /// Stage 3: the first BLAKE2b, and the Stratum-facing half.
    ///
    /// The source notes these are exactly the fields sent to machines over
    /// Stratum v1 — the leading zero word and `h2` play the part of `coinb1`,
    /// and the extranonce sits where an extranonce goes:
    ///
    /// ```text
    ///     ss << (uint32_t)0;   // Final 3 bytes are part of Sv1 "coinb1"
    ///     ss << h2_hash;       // Remainder of Sv1 "coinb1"
    ///     ss << m_extranonce;  // Sv1 "extranonce"
    ///     Assert(ss.size() == 52);
    /// ```
    fn stage3(h2: &[u8; 32], extranonce: &[u8; 16]) -> [u8; 32] {
        let mut input = Vec::with_capacity(52);
        input.extend_from_slice(&0u32.to_le_bytes());
        input.extend_from_slice(h2);
        input.extend_from_slice(extranonce);
        debug_assert_eq!(input.len(), 52);

        blake2b::blake2b_256(&input)
    }

    /// Runs stages 4 and 5 — the part a miner repeats.
    ///
    /// This is the hot loop: one BLAKE2b over at most 160 bytes, then a mask.
    /// Reached through [`PowMidstate::hash`], which picks this or SHA-256d.
    pub fn hash(&self, nonce: u32, nonce2: u32, nonce3: u32, time_offset: u32) -> Hash256 {
        let (n, n2) = (nonce.to_le_bytes(), nonce2.to_le_bytes());
        let (n3, offset) = (nonce3.to_le_bytes(), time_offset.to_le_bytes());

        // --- stage 4: the second BLAKE2b, in one of four arrangements ------
        //
        // Four layouts exist so that hardware from different vendors can be fed
        // the shape it already expects, rather than every vendor having to
        // agree on one.
        let mut input = Vec::with_capacity(160);
        match self.flags & 3 {
            0 => {
                // The previous block, hidden and partly blanked.
                let mut hidden = self.prev_hidden;
                hidden[..6].fill(0);
                input.extend_from_slice(&hidden);
                input.extend_from_slice(&n);
                input.extend_from_slice(&n2);
                input.extend_from_slice(&offset);
                input.extend_from_slice(&n3);
                input.extend_from_slice(&self.stage3);
            }
            1 => {
                input.extend_from_slice(&n);
                input.extend_from_slice(&n2);
                input.extend_from_slice(&n3);
                input.extend_from_slice(&offset);
                input.extend_from_slice(&self.stage3);
                input.extend_from_slice(&self.h2);
            }
            case => {
                // Case 3 is case 2 with a further 32 zero bytes in front.
                if case == 3 {
                    input.extend_from_slice(&[0u8; 32]);
                }
                input.extend_from_slice(&[0u8; 48]);
                input.extend_from_slice(&self.h2);
                input.extend_from_slice(&n);
                input.extend_from_slice(&n2);
                input.extend_from_slice(&offset);
                input.extend_from_slice(&n3);
                input.extend_from_slice(&self.stage3);
            }
        }

        let digest = blake2b::blake2b_256(&input);

        // --- stage 5: mask, and store reversed -----------------------------
        //
        // The node writes the result backwards, so the masked digest is the
        // *display* form and the reversal is its internal order. Getting this
        // the wrong way round yields a hash that looks entirely plausible.
        let mut masked = [0u8; 32];
        for i in 0..32 {
            masked[i] = digest[i] ^ self.xor_mask[i];
        }

        Hash256::from_display_bytes(masked)
    }
}

/// Computes a header's proof-of-work hash.
///
/// Convenience over [`PowMidstate`], for validating a block rather than mining
/// one. A miner should build the midstate once and sweep nonces against it.
pub fn pow_hash(header: &BlockHeader) -> Hash256 {
    PowMidstate::new(header).hash(
        header.nonce,
        header.nonce2,
        header.nonce3,
        header.time_offset,
    )
}
