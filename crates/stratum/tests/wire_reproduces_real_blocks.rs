//! The protocol's whole obligation, checked against real blocks.
//!
//! A Stratum job is a lossy view of a header: it carries two digests and a
//! flags byte, and deliberately withholds the version, the timestamp, the
//! merkle root, and the real previous block hash. The question that decides
//! whether the protocol is right is therefore not "does it round-trip" — it is
//! **does what survives still hash to the same value the node computed**.
//!
//! These tests take headers off the live regtest chain, reduce them to jobs,
//! push those jobs through JSON, rebuild a miner-side midstate from what comes
//! out, and compare against the hash the node stored.

use btcb2_primitives::{BlockHeader, PowMidstate, pow_hash};
use stratum::Job;
use stratum::job::{EXTRANONCE1_SIZE, EXTRANONCE2_SIZE};

/// `(height, serialised header, hash as the node reports it)`.
///
/// Post-fork blocks only: a v1 header has no Stratum representation here,
/// because this protocol describes the BLAKE2b pipeline and a v1 block does
/// not use it.
const BLOCKS: &[(u32, &str, &str)] = &[
    (10, "000000a00418ed1ecbfa8008a908a978605e50e485f3bc6a4a51d5e0114bfe47400c837a4a2fd324fde32e577cc21a1eb7c0593765857f57e7033916368befd7f64c353b21509d6affff7f20000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000000000",
        "250c77d7227d48cfb8e74e7ae3d700044fc56a2be3984bf34230434167a48209"),
    (11, "000000a00982a46741433042f34b98e32b6ac54f0400d7e37a4ee7b8cf487d22d7770c25a95f1b3d8824e10d92a4a89d56f5765f35236acec8ddca37e62a3a5a6c36364921509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000b0000000000000000000000000000000000000000000000000000000000000000000000",
        "74f70f8701988e65966d9267ab49f276848385a1917ef204b10a89d4a9de06aa"),
    (14, "000000a0e39a58a50808ca23cf5960b3f15109a6b0f9425f4721aafee157b1bf01e0823789fa417ec63fd7fb918778ebcd58fcc8d0c6cd81706eec000b59f2b0d5930b2622509d6affff7f20010000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000000000000",
        "21e30e02fa0a31bfb455f8a0b64ecee1b110cf95737b29995fce574da8f5697a"),
];

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// Reduces a header to the job a pool would send for it.
fn job_for(header: &BlockHeader) -> Job {
    let PowMidstate::Blake2b(midstate) = PowMidstate::new(header) else {
        panic!("these vectors are all post-fork");
    };

    Job {
        job_id: "1".to_owned(),
        prev_hidden: midstate.hidden_prev_block(),
        consensus_digest: *midstate.consensus_digest(),
        flags: header.flags,
        bits: header.bits,
        clean_jobs: true,
    }
}

/// The gate: a header reduced to a job, serialised, parsed, and rebuilt by a
/// miner must hash to what the node stored.
///
/// Everything the protocol drops is dropped here too. If any of it were
/// actually needed, this would fail.
#[test]
fn a_job_carries_enough_to_reproduce_the_block_hash() {
    for (height, raw, expected) in BLOCKS {
        let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
        assert_eq!(pow_hash(&header).to_string(), *expected, "vector at {height}");

        // Pool side: reduce to a job and put it on the wire.
        let line = serde_json::to_string(&job_for(&header).to_notify_params()).expect("encodes");

        // Miner side: parse it back and rebuild the midstate from scratch.
        let params: serde_json::Value = serde_json::from_str(&line).expect("decodes");
        let job = Job::from_notify_params(&params).expect("parses");

        let (extranonce1, extranonce2) = header.extranonce.split_at(EXTRANONCE1_SIZE);
        let midstate = job.midstate(extranonce1, extranonce2).expect("halves fit");

        assert_eq!(
            midstate
                .hash(header.nonce, header.nonce2, header.nonce3, header.time_offset)
                .to_string(),
            *expected,
            "height {height}: the job lost something the hash needed"
        );
    }
}

/// The pool must be able to check a share without trusting the miner, which
/// means the same job and the same nonces must give the same hash on both
/// sides — and different nonces must not.
#[test]
fn the_pool_can_reproduce_what_a_miner_claims() {
    let (_, raw, expected) = BLOCKS[0];
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
    let job = job_for(&header);

    let (extranonce1, extranonce2) = header.extranonce.split_at(EXTRANONCE1_SIZE);
    let midstate = job.midstate(extranonce1, extranonce2).expect("halves fit");

    assert_eq!(midstate.hash(header.nonce, 0, 0, 0).to_string(), expected);

    // A miner that lied about its nonce cannot produce the same hash — which is
    // what makes the pool's re-check meaningful rather than ceremonial.
    assert_ne!(
        midstate.hash(header.nonce.wrapping_add(1), 0, 0, 0).to_string(),
        expected
    );
}

/// The extranonce is split the way Stratum splits it, and both halves have to
/// reach the hash — otherwise two miners handed different `extranonce1` values
/// would search the very same space.
#[test]
fn both_extranonce_halves_reach_the_hash() {
    let (_, raw, _) = BLOCKS[0];
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");
    let job = job_for(&header);

    assert_eq!(EXTRANONCE1_SIZE + EXTRANONCE2_SIZE, 16);

    let baseline = job
        .midstate(&[0u8; EXTRANONCE1_SIZE], &[0u8; EXTRANONCE2_SIZE])
        .expect("halves fit")
        .hash(0, 0, 0, 0);

    let mut moved_pool_half = [0u8; EXTRANONCE1_SIZE];
    moved_pool_half[0] = 1;
    assert_ne!(
        job.midstate(&moved_pool_half, &[0u8; EXTRANONCE2_SIZE])
            .expect("halves fit")
            .hash(0, 0, 0, 0),
        baseline,
        "extranonce1 must affect the hash, or miners would collide"
    );

    let mut moved_miner_half = [0u8; EXTRANONCE2_SIZE];
    moved_miner_half[0] = 1;
    assert_ne!(
        job.midstate(&[0u8; EXTRANONCE1_SIZE], &moved_miner_half)
            .expect("halves fit")
            .hash(0, 0, 0, 0),
        baseline,
        "extranonce2 must affect the hash, or a miner could not roll it"
    );
}

/// All four stage-4 layouts have to survive the wire, because the flags byte
/// is what selects between them and it travels in a slot Bitcoin used for the
/// block version.
#[test]
fn every_layout_survives_the_wire() {
    let (_, raw, _) = BLOCKS[0];
    let header = BlockHeader::deserialize(&unhex(raw)).expect("parses");

    let mut hashes = Vec::new();

    for flags in 0..4u8 {
        let mut header = header;
        header.flags = flags;

        let job = job_for(&header);
        let params = job.to_notify_params();
        let parsed = Job::from_notify_params(&params).expect("parses");
        assert_eq!(parsed.flags, flags);

        let (en1, en2) = header.extranonce.split_at(EXTRANONCE1_SIZE);
        let direct = pow_hash(&header);
        let over_the_wire = parsed.midstate(en1, en2).expect("halves fit").hash(
            header.nonce,
            header.nonce2,
            header.nonce3,
            header.time_offset,
        );

        assert_eq!(over_the_wire, direct, "layout {flags} did not survive");
        hashes.push(direct);
    }

    // The layouts exist so different hardware can be fed the shape it expects.
    // They are different arrangements of the same bytes, so they must produce
    // different hashes — if two agreed, one of them is not being built.
    for (i, a) in hashes.iter().enumerate() {
        for (j, b) in hashes.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "layouts {i} and {j} produced the same hash");
        }
    }
}
