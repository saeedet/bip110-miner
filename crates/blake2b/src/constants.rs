//! The two constant tables that define BLAKE2b, from RFC 7693.

/// Initialisation vector `IV[0..8]`, RFC 7693 §2.6.
///
/// Identical to SHA-512's: the first 64 bits of the fractional parts of the
/// square roots of the first eight primes. "Nothing up my sleeve" numbers —
/// derived from small primes so that nobody could have chosen them to hide a
/// weakness.
pub const IV: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

/// Message word permutation `SIGMA`, RFC 7693 §2.7.
///
/// Each round mixes the sixteen message words in a different order, so no word
/// keeps a privileged position. There are only ten rows for twelve rounds:
/// rounds 10 and 11 reuse rows 0 and 1, which the spec states explicitly.
///
/// This is the whole of BLAKE2b's message schedule. Unlike SHA-256 it derives
/// no new words — it only permutes the sixteen it was given, which is part of
/// why it is cheap in software.
pub const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// Bytes consumed per compression. BLAKE2b works on 128-byte blocks — twice
/// SHA-256's, which is one reason it moves more data per unit of work.
pub const BLOCK_BYTES: usize = 128;

/// Rounds per compression.
pub const ROUNDS: usize = 12;
