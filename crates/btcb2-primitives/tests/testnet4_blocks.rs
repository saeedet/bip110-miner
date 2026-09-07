//! Real blocks from the public testnet4 BLAKE2b chain.
//!
//! Everything else this crate is tested against was mined on a regtest chain by
//! this project's own code, which makes those tests a check of internal
//! consistency: if the same misreading were baked into both the miner and the
//! verifier, they would agree and both be wrong.
//!
//! These blocks were mined by somebody else, on a public network, with an
//! independent implementation. Reproducing their hashes is the first evidence
//! that this crate reads the consensus rules the way the rest of the network
//! does, rather than merely the way the rest of this repository does.
//!
//! Captured from a synced node on 2026-09-07 with
//! `getblockheader <hash> false`; regenerate with `scripts/capture-vectors.sh`.

use btcb2_primitives::{BlockHeader, PowMidstate, pow_hash};

/// `(height, serialised header, hash as the node reports it)`.
///
/// 150,308 is the activation height itself — the first block on this network
/// to use a 164-byte header and the BLAKE2b pipeline. Its parent is a
/// SHA-256d block, which makes it the exact seam the fork turns on.
const BLOCKS: &[(u32, &str, &str)] = &[
    (150308, "000000a0ccb157caa788400a667f6c19858ee913c701a42c8d1cd85122ec17000000000043d2e57990429ae581621ce01aa5fbf5e4c2723996be18660a4930b91e96d6c871b4946affff001dce0ac801d123881f71b4946a00000000b10cf00d0100000000000000000000008e00000000000000000000000000000000000000244b02000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000b9d1b7e1bb0e77215ee92c6ef7ec8f4473e23908380649e779b6"),
    (150309, "000000a0b679e74906380839e273448fecf76e2ce95e21770ebbe1b7d1b9000000000000b89a9a63a3e422bc9e7a866a2979dcdb4f4a63df0958fa798d3645fbfb1cac1487b9946affff001d73541009b64dc60f87b9946a00000000b10cf00d0100000000000000000000002200000000000000000000000000000000000000254b02000000000000000000000000000000000000000000000000000000000000000000",
        "00000000000096ebd9ecd086095f024d8eb4d1cdb9d8b124dc0e610a4f24f858"),
    (150400, "000000a0765c69a2cbd798aa84a75627556feed37237f12fa2b660b719c0606c000000007809323cd85a9052433d65a589c25a6fcd208073cbe0993055bccf90ae887b2ec2379a6affff001dd0a04b00000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000804b02000000000000000000000000000000000000000000000000000000000000000000",
        "00000000c4ad4d820915ea836c9a174282f0a5192278e2b2b017f674d3406df6"),
    (150500, "000000a095acf3552c419c84919729a1333ec1a7d8a48496c98cd2b1d650fb8d000000006a28023f13f1fee323ba10407e28ade556eb922d45b84b4f7e1ca736f81003cb845f9c6affff001df3810a00000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000e44b02000000000000000000000000000000000000000000000000000000000000000000",
        "00000000d5222b469c1eccd401928b543112d36cfee64690c23863c6a67d1e82"),
    (150613, "000000a026aaf4548dd76eb008f34d1ec7802e7fe359d7898587cb479192ec0e000000004933601b7cbe282add93f7997a6fff845a6a1660a53984a383886848124bed051eb69e6affff001d53dd4a00000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000554c02000000000000000000000000000000000000000000000000000000000000000000",
        "0000000007c568720f000801422afb787c4ebd61b205f6dc40582a2b0e36a159"),
    (150614, "000000a059a1360e2b2a5840dcf605b261bd4e7c78fb2a420108000f7268c50700000000a22acd3b91703370b08dab3fef5caf0571844a29b6ee5dac8398b84fb4a63973f4ba9e6affff001d9e5d3800000000000000000000000000000000000000000000000000000000001d00000000000000000000000000000000000000564c02000000000000000000000000000000000000000000000000000000000000000000",
        "000000000796618858f0b4be27d88ce4c62fca27a6f868fea50f221aff55f49c"),
    // Ours, mined 2026-09-07 14:04 by this project's pool and miner, and
    // accepted by the independent node at 82.67.102.15 as its new tip. Kept
    // here as a vector like any other: read back off the chain rather than
    // from what we thought we submitted.
    (150616, "000000a0146b78be5b70e6ab5362f9905e3fbf8b323cee7e39f6e23222ef8e4e000000009e03b829b553687d72b511e3f208804b209cc2a53ca9d487db8e45a6e86bbd235cc49e6affff001dc9b0fa04000000000000000000000000000000010000000000000002000000000200000000000000000000000000000000000000584c02000000000000000000000000000000000000000000000000000000000000000000",
        "0000000063400d7051d4049e1cd43f0d068c52768d76d41f88bc043bc5cca40e"),
];

/// The block this project mined, read back off the public chain.
///
/// Two details worth pinning, because both are claims the rest of the code
/// makes and this is where the network agrees with them:
///
/// * the extranonce is `…0001` followed by `…0002` — the pool's assigned half
///   and the miner thread's own, spliced into one 16-byte header field, which
///   is the whole point of Stratum's extranonce split
/// * `nonce2` and `nonce3` are zero, because `mining.submit` has no slot for
///   them and the worker deliberately never rolls them
const OURS: u32 = 150616;

#[test]
fn our_own_block_reads_back_correctly() {
    let (_, raw, hash) = BLOCKS.iter().find(|(h, _, _)| *h == OURS).expect("present");
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

    assert_eq!(pow_hash(&header).to_string(), *hash);
    assert_eq!(header.height, OURS);
    assert_eq!(header.bits, 0x1d00_ffff, "mined in the minimum-difficulty window");

    assert_eq!(
        header.extranonce,
        [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2],
        "extranonce1 from the pool, extranonce2 from the miner"
    );
    assert_eq!((header.nonce2, header.nonce3), (0, 0), "no Stratum slot for these");
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// The gate: our five stages must reproduce hashes computed by another
/// implementation, on a network this code has never mined on.
#[test]
fn pow_reproduces_public_testnet4_blocks() {
    for (height, raw, expected) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

        assert!(header.header_v2, "height {height} should be post-fork");
        assert_eq!(pow_hash(&header).to_string(), *expected, "height {height}");
    }
}

/// A header must survive a parse and re-serialise unchanged, or we are reading
/// a field the network writes differently.
#[test]
fn headers_round_trip() {
    for (height, raw, _) in BLOCKS {
        let bytes = unhex(raw);
        let header = BlockHeader::deserialize(&bytes).expect("parses");

        assert_eq!(bytes.len(), 164, "height {height}");
        assert_eq!(header.serialize(), bytes, "height {height}");
    }
}

/// The header commits to its own height, and the node agrees.
///
/// A field this crate could have byte-swapped or misplaced and still produced
/// plausible-looking output, so it is worth checking against a value we know
/// independently.
#[test]
fn the_committed_height_matches_the_chain() {
    for (height, raw, _) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
        assert_eq!(header.height, *height);
    }
}

/// Real miners on this chain use layout 0 and no XOR key, which is what this
/// project's pool also produces.
///
/// Not a consensus rule — any of the four layouts is valid, and a pool hiding
/// blocks from its miners would set a key. Worth pinning because it says the
/// defaults chosen here match what the network actually runs.
#[test]
fn the_network_mines_layout_zero_unmasked() {
    for (height, raw, _) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

        assert_eq!(header.flags & 3, 0, "height {height}: stage-4 layout");
        assert_eq!(header.xor_key, [0u8; 16], "height {height}: XOR key");
        assert_eq!(PowMidstate::new(&header).nonce_words(), 4);
    }
}

/// Every one of these was mined at difficulty 1.
///
/// testnet4 allows minimum-difficulty blocks 20 minutes after their parent,
/// and on the BLAKE2b side of the fork there is little enough hashpower that
/// the rule applies to essentially every block. That is what makes a CPU a
/// credible miner here — and what a `bits` value of anything else would
/// disprove.
#[test]
fn the_chain_runs_at_minimum_difficulty() {
    for (height, raw, _) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
        assert_eq!(header.bits, 0x1d00_ffff, "height {height}");
    }
}
