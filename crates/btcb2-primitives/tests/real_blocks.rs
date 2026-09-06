//! Verification against real blocks from a live chain.
//!
//! Every value below was pulled from a running Knots node with BLAKE2b active,
//! not constructed by hand. That matters: a header format and a five-stage hash
//! pipeline are both easy to transcribe *almost* correctly, and only real data
//! catches the difference.
//!
//! Regenerate with `scripts/capture-vectors.sh` if the chain is rebuilt.

use btcb2_primitives::{BlockHeader, pow_hash};
use std::str::FromStr;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// Real headers: `(height, raw hex, block hash)`.
const BLOCKS: &[(u32, &str, &str)] = &[
    (8, "000000200ce92bd02605e73da4f6e6ad6d2e1046b0fb1401d850d413fd6fe3f8a2d5e3691ea46c666506c12a7a62dcdf38d305c2ebf73ee9afbb497525cf6af1d700372e21509d6affff7f2000000000",
        "26f32a0550e5e1c29d17927d0b81c05e62282df12d8323c9a119a571280e10ad"),
    (9, "00000020ad100e2871a519a1c923832df12d28625ec0810b7d92179dc2e1e550052af3262235169ed79cb95af7eb0e0ede83a9953c58973e738ea589dcff42168c0d667821509d6affff7f2000000000",
        "7a830c4047fe4b11e0d5514a6abcf385e4505e6078a908a90880facb1eed1804"),
    (10, "000000a00418ed1ecbfa8008a908a978605e50e485f3bc6a4a51d5e0114bfe47400c837a4a2fd324fde32e577cc21a1eb7c0593765857f57e7033916368befd7f64c353b21509d6affff7f20000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000000000",
        "250c77d7227d48cfb8e74e7ae3d700044fc56a2be3984bf34230434167a48209"),
    (11, "000000a00982a46741433042f34b98e32b6ac54f0400d7e37a4ee7b8cf487d22d7770c25a95f1b3d8824e10d92a4a89d56f5765f35236acec8ddca37e62a3a5a6c36364921509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000b0000000000000000000000000000000000000000000000000000000000000000000000",
        "74f70f8701988e65966d9267ab49f276848385a1917ef204b10a89d4a9de06aa"),
    (12, "000000a0aa06dea9d4890ab104f27e91a185838476f249ab67926d96658e9801870ff7748a3af539a2a4ec4465d6216bd5d9146a36f6fab3558f16870dd001e93a7b45ea21509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000",
        "1fc1787c2f2db3b8f3335cfb8c67579e27d598d060b38f0bdc150a40c55ddf1d"),
    (13, "000000a01ddf5dc5400a15dc0b8fb360d098d5279e57678cfb5c33f3b8b32d2f7c78c11fa65c856b58d79003ee590b3c169d7e1fcce95a9545542bcf4309b5e5185bc4f821509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000d0000000000000000000000000000000000000000000000000000000000000000000000",
        "3782e001bfb157e1feaa21475f42f9b0a60951f1b36059cf23ca0808a5589ae3"),
    (14, "000000a0e39a58a50808ca23cf5960b3f15109a6b0f9425f4721aafee157b1bf01e0823789fa417ec63fd7fb918778ebcd58fcc8d0c6cd81706eec000b59f2b0d5930b2622509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000000000000",
        "21e30e02fa0a31bfb455f8a0b64ecee1b110cf95737b29995fce574da8f5697a"),
];

/// Every header must survive a parse and re-serialise unchanged.
///
/// Covers both formats: blocks below the activation height are 80-byte v1
/// headers, those above are 164-byte v2. A round trip proves no field was
/// dropped, reordered, or silently widened.
#[test]
fn headers_round_trip() {
    for (height, raw, _) in BLOCKS {
        let bytes = unhex(raw);
        let header = BlockHeader::deserialize(&bytes)
            .unwrap_or_else(|e| panic!("height {height} failed to parse: {e}"));

        assert_eq!(
            hex(&header.serialize()),
            *raw,
            "height {height} did not re-serialise identically"
        );
    }
}

/// The activation boundary is visible in the header size itself.
#[test]
fn the_fork_is_visible_in_the_format() {
    for (height, raw, _) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

        if *height < 10 {
            assert!(!header.header_v2, "height {height} should be a v1 header");
            assert_eq!(raw.len() / 2, 80);
        } else {
            assert!(header.header_v2, "height {height} should be a v2 header");
            assert_eq!(raw.len() / 2, 164);
            assert_eq!(header.height, *height, "v2 headers commit to their height");
        }
    }
}

/// **The phase gate**: the five-stage pipeline reproduces real block hashes.
///
/// Passing this means the header parse, three BIP340 tagged hashes, both
/// BLAKE2b passes, the flag-selected layout, the XOR mask and the byte order
/// are all correct *simultaneously*. There is no way to arrive at these values
/// with any one of them wrong.
#[test]
fn pow_reproduces_real_block_hashes() {
    let mut checked = 0;

    for (height, raw, expected) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

        // v1 blocks are SHA-256d, which this crate does not implement — it is
        // a BLAKE2b miner. They are here for the round-trip tests above.
        if !header.header_v2 {
            continue;
        }

        assert_eq!(
            pow_hash(&header).to_string(),
            *expected,
            "height {height}"
        );
        checked += 1;
    }

    assert!(checked >= 5, "expected several BLAKE2b blocks, checked {checked}");
}

/// A header parsed from real data must hash the same through the convenience
/// entry point and through an explicitly reused midstate.
///
/// This is the property a miner depends on: stages 0-3 computed once, then
/// thousands of nonces run against them.
#[test]
fn midstate_reuse_matches_the_one_shot_path() {
    use btcb2_primitives::PowMidstate;

    let (_, raw, expected) = BLOCKS.iter().find(|(h, _, _)| *h == 10).expect("block 10");
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

    let midstate = PowMidstate::new(&header);
    let from_midstate = midstate.hash(
        header.nonce,
        header.nonce2,
        header.nonce3,
        header.time_offset,
    );

    assert_eq!(from_midstate.to_string(), *expected);

    // And a different nonce against the same midstate must give something else,
    // or the nonce is not reaching the hash at all.
    let other = midstate.hash(header.nonce.wrapping_add(1), header.nonce2, header.nonce3, header.time_offset);
    assert_ne!(other, from_midstate, "the nonce must affect the hash");
}

/// A truncated header must error rather than panic.
#[test]
fn truncated_headers_are_rejected() {
    let (_, raw, _) = BLOCKS.last().expect("at least one block");
    let bytes = unhex(raw);

    for length in 0..bytes.len() {
        assert!(
            BlockHeader::deserialize(&bytes[..length]).is_err(),
            "a {length}-byte prefix should not parse"
        );
    }
}

/// A hash prints in display order and parses back to the same value.
#[test]
fn hash_display_round_trips() {
    let (_, _, expected) = BLOCKS.last().expect("at least one block");
    let hash = btcb2_primitives::Hash256::from_str(expected).expect("valid hash");
    assert_eq!(hash.to_string(), *expected);
}
