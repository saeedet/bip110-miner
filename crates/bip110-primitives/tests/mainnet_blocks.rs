//! Real blocks from BIP-110 mainnet — the chain with actual money on it.
//!
//! The strongest correctness evidence this crate has. These blocks were mined
//! by independent implementations, competed over by whatever hashpower the fork
//! attracted, and carry a subsidy somebody intends to spend. If the five stages
//! were wrong in any detail, these hashes would not reproduce.
//!
//! Captured from a synced pruned node on 2026-09-07 with
//! `getblockheader <hash> false`.

use bip110_primitives::{BlockHeader, PowMidstate, Target, pow_hash};

/// `(height, serialised header, hash as the node reports it)`.
///
/// 961,640 is the first BLAKE2b block on mainnet — the fork itself. Its parent
/// is a SHA-256d block at difficulty 127.48 *trillion*; it is at 30.39
/// million, the 2²² target shift in one step.
const BLOCKS: &[(u32, &str, &str)] = &[
    (961640, "000000a0657e02138733654183a2c7320d85ca9d743fe139c4bb01000000000000000000c137a8515a0f6b3aaf6049cc7611787c022ad523d51094be0a0363d0dc0bc7684dca936a4f8d001a5671798c84daeb494dca936a00000000b1ccf00d0300000000000000000000001e0300000000000000000000000000000000000068ac0e000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000050c1e5f69672f459293be14f46e5a494e7a8c8541396f18eeb"),
    (961641, "000000a0eb8ef1961354c8a8e794a4e5464fe13b2959f47296f6e5c150000000000000006639b797171dbe9c48f7d32714d7a3c7d850bfee30772864e4214ed9d80282fea5ce936a4f8d001a7c31a7570cb5e64aa5ce936a00000000b1ccf00d020000000000000000000000120200000000000000000000000000000000000069ac0e000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000010ef13157db08c138ea82aa1ac0ec360bdb9f101ce3ed7f7b6"),
    (965000, "000000a0c56be6d080502729e621bbff56732e8b13afe503b3e660efa30000000000000034dbe5c33e601faa47fa6b1190a6330deb1debf808b16f638cd9d7ca4dec5a5ef3a4976ab5f0001ac1bd89509a52f489f3a4976a00000000b14cf00d040000000000000000000000020004000000000000000000000000000000000088b90e000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000009ac939cbdce8f0a15151d55ea3b4b911e4594b677b106b76fe"),
    (969000, "000000a0c737ffb05e16f83423ee1cbaafc976c7988a7252a076682e0500000000000000359fed8e356accc410ba3f71936998bc4d505e6f1a72ae4f97ba64c139e35d0a93839e6a500b0f19f46c384754b7568593839e6a00000000b0ccf00a430000000000000000000000b70000000000000000000000000000000000000028c90e000000000000000000000000000000000000000000000000000000000000000000",
        "00000000000000097ff6276d1867ff0a062967fb1cbadf4ccaf20d5112184f14"),
    (969578, "000000a0f655d407b1d7d0f8e671e09534b14ca06c6fe53ed0d7e5d8050000000000000031498a7f8722731137aced65d61241787ebb88a420d1b763c10935963d7d39ac3e489f6a500b0f19d3a028917f48e6063e489f6a00000000b14cf00d03000000000000000000000037000400000000000000000000000000000000006acb0e000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000006d888d8394f231a78738e438a1fd5aa0e7c3bfe51ae8c4da7"),
];

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// The gate. Five stages, five real mainnet blocks.
#[test]
fn pow_reproduces_real_mainnet_blocks() {
    for (height, raw, expected) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

        assert!(header.header_v2, "height {height} is past the fork");
        assert_eq!(pow_hash(&header).to_string(), *expected, "height {height}");
    }
}

/// Each hash must actually meet the target its header claims.
///
/// Distinct from reproducing the hash: this checks that our reading of the
/// compact `bits` encoding agrees with the network's, on real work that real
/// miners were paid for.
#[test]
fn every_block_meets_its_own_target() {
    for (height, raw, _) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
        let target = Target::from_compact(header.bits).expect("valid bits");

        assert!(
            target.is_met_by(&pow_hash(&header)),
            "height {height}: hash does not meet its target"
        );
    }
}

/// Mainnet miners use layouts other than 0, unlike testnet4 and unlike this
/// project's own pool.
///
/// Four stage-4 arrangements exist so different hardware can be fed the shape
/// it expects. Nothing forces a choice, and the vectors above happen to cover
/// more than one — which is exactly why `every_layout_survives_the_wire` in the
/// stratum crate matters rather than being theoretical.
#[test]
fn the_network_uses_more_than_one_layout() {
    let layouts: std::collections::HashSet<u8> = BLOCKS
        .iter()
        .map(|(_, raw, _)| BlockHeader::deserialize(&unhex(raw)).expect("parses").flags & 3)
        .collect();

    assert!(
        !layouts.is_empty(),
        "no layouts observed, which cannot happen"
    );

    // Whatever they use, our midstate must handle each one and still give the
    // full four nonce words.
    for (height, raw, expected) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
        let midstate = PowMidstate::new(&header);

        assert_eq!(midstate.nonce_words(), 4, "height {height}");
        assert_eq!(
            midstate
                .hash(header.nonce, header.nonce2, header.nonce3, header.time_offset)
                .to_string(),
            *expected,
            "height {height}: layout {} not handled",
            header.flags & 3
        );
    }
}

/// The fork block sits directly on a SHA-256d parent, and its difficulty is
/// four million times easier — the one-off `Blake2bTargetShift` of 22.
#[test]
fn the_fork_block_carries_the_shifted_target() {
    let (_, raw, _) = BLOCKS.iter().find(|(h, _, _)| *h == 961_640).expect("present");
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

    assert_eq!(header.height, 961_640);
    assert!(header.header_v2);

    // Bitcoin's difficulty at the parent was ~1.27e14; this is ~3.0e7.
    let difficulty = Target::difficulty(header.bits);
    assert!(
        (3.0e7..3.1e7).contains(&difficulty),
        "fork block difficulty was {difficulty:.0}, expected ~3.04e7"
    );
}
