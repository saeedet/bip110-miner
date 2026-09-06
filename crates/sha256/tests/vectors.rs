//! Known-answer tests for SHA-256 and BIP340 tagged hashes.

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The NIST vectors, which pin the compression function and the padding.
#[test]
fn nist_sha256_vectors() {
    assert_eq!(
        hex(&sha256::sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// Tagged hashes, including the three tags this chain's proof of work uses.
#[test]
fn bip340_tagged_hashes() {
    assert_eq!(
        hex(&sha256::tagged_hash(b"BIP0340/aux", b"")),
        "07fab5f97e680abb8389d1fa164281e124439468f5bd699fcbd1ae86e6405d69"
    );
    assert_eq!(
        hex(&sha256::tagged_hash(b"Bitcoin block header 1", b"")),
        "aa38b35fb8482096f9de807f9f5d2899421fec9373ac823fdd380424d4211c54"
    );
    assert_eq!(
        hex(&sha256::tagged_hash(b"Merge-mining hook", &[0u8; 32])),
        "afdd7dad27a55d6ce26f039268d73ae9b5a5c9116e5b46bc9bcef5a6181915c2"
    );
    assert_eq!(
        hex(&sha256::tagged_hash(b"Bitcoin block hash PoW XOR key", &[0u8; 16])),
        "86e4855b51daf0932719011a6565a5908aef105fc6f8b85a23601de43865f4db"
    );
}

/// Different tags over identical data must not collide. This is the entire
/// point of tagging, so it is worth asserting rather than assuming.
#[test]
fn tags_domain_separate() {
    assert_ne!(
        sha256::tagged_hash(b"tag one", b"same data"),
        sha256::tagged_hash(b"tag two", b"same data"),
    );
}

/// A tagged hash is not a plain hash of tag and data concatenated.
#[test]
fn tagging_is_not_concatenation() {
    assert_ne!(
        sha256::tagged_hash(b"abc", b"def"),
        sha256::sha256(b"abcdef"),
    );
}

/// The incremental builder must agree with the one-shot form, and must report
/// the byte counts the node asserts on.
#[test]
fn builder_matches_one_shot() {
    let mut hasher = sha256::TaggedHasher::new(b"Merge-mining hook");
    assert_eq!(hasher.bytes_written(), 0x40, "the prefix is exactly two blocks' worth");

    hasher.write(&[0u8; 32]);
    assert_eq!(hasher.bytes_written(), 0x40 + 32);

    assert_eq!(
        hasher.finalize(),
        sha256::tagged_hash(b"Merge-mining hook", &[0u8; 32]),
    );
}

/// Field order must matter — a hasher that ignored it would silently accept a
/// header assembled wrongly.
#[test]
fn field_order_matters() {
    let mut a = sha256::TaggedHasher::new(b"t");
    a.write_u32(1).write_u8(2);

    let mut b = sha256::TaggedHasher::new(b"t");
    b.write_u8(2).write_u32(1);

    assert_ne!(a.finalize(), b.finalize());
}
