//! A portable BLAKE2b, written to mirror RFC 7693 line by line.
//!
//! Not fast, and not trying to be. Its job is to be checkable against the
//! published specification by eye, so that it can serve as the *definition* of
//! correctness the optimised version is tested against.
//!
//! # How it differs from SHA-256
//!
//! Both are Merkle–Damgård-ish constructions over a compression function, but
//! the details point in opposite directions:
//!
//! - **64-bit words** rather than 32-bit, so it moves twice the data per
//!   operation on a 64-bit machine.
//! - **No message schedule.** SHA-256 derives 48 extra words per block;
//!   BLAKE2b only *permutes* the sixteen it was given, via [`SIGMA`].
//! - **Length is a counter, not padding.** The number of bytes so far is fed
//!   into the compression function directly, rather than appended to the
//!   message.
//! - **A finalisation flag** rather than a distinguishing suffix.
//!
//! Together those are why it is fast in software and unattractive to put in
//! silicon — which is precisely why a fork wanting CPU mining would choose it.

use crate::constants::{BLOCK_BYTES, IV, ROUNDS, SIGMA};

/// The mixing function `G`, RFC 7693 §3.1.
///
/// Mixes two message words into four of the sixteen working words. The four
/// rotation distances — 32, 24, 16, 63 — are the whole of its non-linearity
/// budget, chosen so that every one is cheap on a 64-bit machine.
#[allow(clippy::too_many_arguments)]
fn mix(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);

    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);

    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);

    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

/// The compression function `F`, RFC 7693 §3.2.
///
/// `counter` is the total number of bytes fed in *including* this block, and
/// `last` marks the final block. Those two carry the roles that padding and a
/// length suffix play in SHA-256.
pub(crate) fn compress(state: &mut [u64; 8], block: &[u8; BLOCK_BYTES], counter: u128, last: bool) {
    // Read the block as sixteen little-endian words. Little-endian, unlike
    // SHA-256's big-endian — a deliberate choice, since every machine that
    // matters is little-endian and the conversion is then free.
    let mut m = [0u64; 16];
    for (i, word) in m.iter_mut().enumerate() {
        let bytes: [u8; 8] = block[i * 8..i * 8 + 8].try_into().expect("8-byte slice");
        *word = u64::from_le_bytes(bytes);
    }

    // Sixteen working words: the state, then the IV.
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(state);
    v[8..].copy_from_slice(&IV);

    // Fold in the byte counter, and invert v[14] on the final block. This is
    // what stops a prefix of a message hashing the same as the message.
    v[12] ^= counter as u64;
    v[13] ^= (counter >> 64) as u64;
    if last {
        v[14] = !v[14];
    }

    for round in 0..ROUNDS {
        // Twelve rounds over ten permutations: rounds 10 and 11 reuse rows 0
        // and 1, exactly as the spec prescribes.
        let s = SIGMA[round % SIGMA.len()];

        // Four columns, then four diagonals.
        mix(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        mix(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        mix(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        mix(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);

        mix(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        mix(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        mix(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        mix(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }

    // Feed-forward: XOR both halves back into the state. As with SHA-256's
    // addition, this is what makes the compression one-way — without it the
    // rounds would be a reversible permutation.
    for i in 0..8 {
        state[i] ^= v[i] ^ v[i + 8];
    }
}

/// Computes an unkeyed BLAKE2b digest of `digest_size` bytes.
///
/// # Panics
///
/// If `digest_size` is zero or above 64, which RFC 7693 does not define.
pub fn hash(message: &[u8], digest_size: usize) -> Vec<u8> {
    assert!(
        (1..=64).contains(&digest_size),
        "BLAKE2b digest size must be 1..=64, got {digest_size}"
    );

    let mut state = IV;

    // The parameter block, XORed into the first state word. For an unkeyed
    // hash only three fields are non-zero: the digest length, and a fanout and
    // depth of 1 meaning "sequential, not a tree".
    //
    // This is why BLAKE2b-256 is not merely a truncated BLAKE2b-512: the
    // length is mixed in at the start, so the two produce unrelated output.
    state[0] ^= 0x0101_0000 ^ (digest_size as u64);

    // Every whole block except the last is compressed as it is passed. The
    // final block is handled below, because it needs the `last` flag — and
    // because a message that is an exact multiple of the block size must still
    // finish with a real block rather than a padded empty one.
    let mut offset = 0;
    while message.len() - offset > BLOCK_BYTES {
        let block: &[u8; BLOCK_BYTES] = message[offset..offset + BLOCK_BYTES]
            .try_into()
            .expect("checked length");
        offset += BLOCK_BYTES;
        compress(&mut state, block, offset as u128, false);
    }

    // The remainder, zero-padded. An empty message still compresses one
    // all-zero block with a counter of zero.
    let mut final_block = [0u8; BLOCK_BYTES];
    let remaining = &message[offset..];
    final_block[..remaining.len()].copy_from_slice(remaining);
    compress(&mut state, &final_block, message.len() as u128, true);

    // Output is the state, little-endian, truncated to the requested length.
    let mut out = Vec::with_capacity(digest_size);
    for word in state {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.truncate(digest_size);
    out
}
