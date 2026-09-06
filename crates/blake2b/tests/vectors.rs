//! Known-answer tests for BLAKE2b.
//!
//! The 512-bit case is RFC 7693's own published vector, which is the strongest
//! evidence available that this implementation is the function the RFC
//! describes. The 256-bit cases are what the chain actually uses.

/// Encodes bytes as lowercase hex. Deliberately dependency-free: this crate has
/// no dependencies, and adding one for tests would spoil that.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 7693 Appendix A: the specification's own worked example.
///
/// If this passes, the compression function, the message permutation, the
/// rotation distances, the counter handling and the finalisation flag are all
/// correct simultaneously — there is no way to land on this value with any of
/// them wrong.
#[test]
fn rfc7693_appendix_a() {
    assert_eq!(
        hex(&blake2b::blake2b_512(b"abc")),
        "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1\
         7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
    );
}

/// The 256-bit digests this chain's proof of work is built on.
#[test]
fn blake2b_256_vectors() {
    assert_eq!(
        hex(&blake2b::blake2b_256(b"")),
        "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"
    );
    assert_eq!(
        hex(&blake2b::blake2b_256(b"abc")),
        "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"
    );
    assert_eq!(
        hex(&blake2b::blake2b_256(b"The quick brown fox jumps over the lazy dog")),
        "01718cec35cd3d796dd00020e0bfecb473ad23457d063b75eff29c0ffa2e58a9"
    );
}

/// Lengths either side of the 128-byte block boundary.
///
/// This is where implementations break: 128 bytes exactly must compress one
/// real block with the final flag set, **not** a full block followed by an
/// empty padded one. Getting that wrong passes every short test and fails
/// silently on real data.
#[test]
fn block_boundary_lengths() {
    let pattern: Vec<u8> = (0..=255u8).collect();

    let cases: [(usize, &str); 4] = [
        (127, "f2fe67ff342e21b8f45e8f2e0bcd1d9243245d50ee6c78042e9c491388791c72"),
        (128, "c3582f71ebb2be66fa5dd750f80baae97554f3b015663c8be377cfcb2488c1d1"),
        (129, "f7f3c46ba2564ff4c4c162da1f5b605f9f1c4aa6a20652a9f9a337c1a2f5b9c9"),
        (256, "39a7eb9fedc19aabc83425c6755dd90e6f9d0c804964a1f4aaeea3b9fb599835"),
    ];

    for (length, expected) in cases {
        assert_eq!(
            hex(&blake2b::blake2b_256(&pattern[..length])),
            expected,
            "disagreement at {length} bytes"
        );
    }

    // Four whole blocks, to exercise the loop rather than just its edges.
    let long: Vec<u8> = pattern.iter().chain(pattern.iter()).copied().collect();
    assert_eq!(
        hex(&blake2b::blake2b_256(&long)),
        "540b20132d8aeae54057cb69c24f95d26a1c472cc700dd450defe9bb796d4f14",
        "disagreement at 512 bytes"
    );
}

/// A 256-bit digest must not be the first 32 bytes of a 512-bit one.
///
/// The digest length is mixed into the initial state, so the two are unrelated.
/// Truncating instead is a plausible-looking mistake that would pass nothing
/// but is worth pinning explicitly.
#[test]
fn shorter_digests_are_not_truncations() {
    let long = blake2b::blake2b_512(b"abc");
    let short = blake2b::blake2b_256(b"abc");
    assert_ne!(&long[..32], &short[..]);
}

/// Every digest length the RFC defines must work.
#[test]
fn all_defined_digest_sizes_work() {
    for size in 1..=64 {
        assert_eq!(blake2b::reference::hash(b"length check", size).len(), size);
    }
}

/// Empty input still compresses one block, with a zero counter.
#[test]
fn empty_input_is_a_full_block_of_zeros() {
    assert_eq!(blake2b::blake2b_256(b"").len(), 32);
    assert_ne!(blake2b::blake2b_256(b""), blake2b::blake2b_256(b"\x00"));
}
