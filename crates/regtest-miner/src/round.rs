//! One attempt at mining a block: template in, submitted block out.
//!
//! Kept separate from `main` so the sequence is readable end to end without
//! argument parsing and setup in the way. Every step here corresponds to
//! something a real miner does; nothing is skipped because it is regtest.

use btcb2_primitives::{BlockHeader, Hash256, PowMidstate, hex};
use mining::{BlockBuilder, CoinbaseBuilder, NonceSpace, search, witness};
use node_rpc::{BlockTemplate, RpcClient};

/// How many hashes to try before returning to check on the chain.
///
/// On regtest a block takes a handful of attempts, so this is never reached.
/// It exists because the nonce space is 2¹²⁸ wide and a search that ran to
/// exhaustion would never return.
const ATTEMPTS_PER_ROUND: u64 = 1 << 24;

/// What happened in one round.
pub enum Outcome {
    /// The node accepted our block.
    Accepted {
        /// The block's proof-of-work hash.
        hash: Hash256,
        /// Its height.
        height: u32,
        /// The nonces that solved it.
        at: NonceSpace,
        /// How many hashes it took.
        hashes: u64,
    },
    /// The node rejected it, and said why.
    Rejected {
        /// The node's reason string — `bad-cb-height`, `high-hash`,
        /// `bad-txnlist-size`, `bad-headline`, and so on.
        reason: String,
    },
    /// The attempt budget ran out without a solution.
    ///
    /// Impossible on regtest, routine anywhere else.
    Exhausted {
        /// The lowest hash seen.
        best: Hash256,
        /// How many leading zero bits it had.
        best_zero_bits: u32,
        /// How many the target demands, for comparison.
        needed_zero_bits: u32,
    },
}

/// Builds a block from `template` and mines it.
pub fn mine(
    client: &RpcClient,
    template: &BlockTemplate,
    payout_script: &[u8],
    extranonce: u64,
    headline: Option<&[u8]>,
    start: NonceSpace,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    // --- 1. The witness commitment ------------------------------------------
    //
    // We derive this ourselves and then check it against the node's own answer.
    // Deriving it is the point; having ground truth to check against is what
    // makes deriving it safe.
    let wtxids: Vec<Hash256> = template
        .transactions
        .iter()
        .map(|tx| tx.wtxid())
        .collect::<Result<_, _>>()?;

    let commitment_script = witness::commitment_script(&wtxids);

    if let Some(expected) = &template.default_witness_commitment {
        let ours = hex::encode(&commitment_script);
        if &ours != expected {
            return Err(format!(
                "our witness commitment disagrees with the node\n  ours: {ours}\n  node: {expected}"
            )
            .into());
        }
    }

    // --- 2. The coinbase ----------------------------------------------------
    //
    // The headline goes in only at the activation height. `is_fork_block` costs
    // one RPC per round, which is nothing next to being wrong about it: too
    // early and the bytes are wasted, too late and the block is rejected.
    let mut coinbase = CoinbaseBuilder::new(
        template.height,
        template.coinbase_value,
        payout_script.to_vec(),
    )
    .extranonce(extranonce.to_le_bytes().to_vec())
    .tag(b"btcb2-miner".to_vec())
    .witness_commitment(commitment_script);

    if let Some(headline) = headline
        && is_fork_block(client, template)?
    {
        coinbase = coinbase.headline(headline.to_vec());
    }

    let coinbase = coinbase.build()?;

    // --- 3. The block -------------------------------------------------------
    let raw_transactions: Vec<Vec<u8>> = template
        .transactions
        .iter()
        .map(|tx| tx.raw())
        .collect::<Result<_, _>>()?;
    let txids: Vec<Hash256> = template
        .transactions
        .iter()
        .map(|tx| tx.txid())
        .collect::<Result<_, _>>()?;

    let builder = BlockBuilder::new(coinbase, raw_transactions, txids)?;

    // The template's contribution to the header. Everything that depends on
    // the block's contents — merkle root, height, txcount — is filled in by
    // `BlockBuilder::header`, and everything a miner grinds starts at zero.
    let from_template = BlockHeader {
        header_v2: template.header_v2(),
        version: template.base_version(),
        prev_block: template.previous_block()?,
        merkle_root: Hash256::ZERO,
        time_on_wire: u32::try_from(template.current_time)?,
        bits: template.compact_bits()?,
        nonce: 0,
        nonce2: 0,
        nonce3: 0,
        extranonce: [0u8; 16],
        time_offset: 0,
        txcount: 0,
        // Layout 0, and no time offset. Leaving `FLAG_USE_TIME_OFFSET` clear is
        // what makes `time_offset` a free nonce word — see `mining::search`.
        flags: 0,
        // No XOR masking. That mechanism exists so a pool can hide a block from
        // its own miners; a solo miner is the pool, and hiding a block from
        // itself would be a strange thing to want.
        xor_key_mask_clear_bits: 0,
        xor_key: [0u8; 16],
        height: 0,
        // No merge mining.
        mm_rhs: Hash256::ZERO,
    };

    let header = builder.header(&from_template, template.height)?;

    // --- 4. The search ------------------------------------------------------
    //
    // Stages 0 to 3 of the proof of work depend on nothing we are about to
    // grind, so they are computed once here and reused for every attempt.
    let target = template.target()?;
    let midstate = PowMidstate::new(&header);
    let result = search(&midstate, &target, start, ATTEMPTS_PER_ROUND);

    let Some(solution) = result.solution else {
        return Ok(Outcome::Exhausted {
            best: result.best,
            best_zero_bits: result.best.leading_zero_bits(),
            needed_zero_bits: target.leading_zero_bits(),
        });
    };

    // --- 5. Submit ----------------------------------------------------------
    let solved = BlockHeader {
        nonce: solution.at.nonce,
        nonce2: solution.at.nonce2,
        nonce3: solution.at.nonce3,
        time_offset: solution.at.time_offset,
        ..header
    };

    // A last check against our own code before bothering the node. If these
    // disagree the bug is in serialisation, and the node's rejection reason
    // would be `high-hash` — which points at the search, the wrong place.
    debug_assert_eq!(btcb2_primitives::pow_hash(&solved), solution.hash);

    let raw_block = builder.serialize(&solved);

    match client.submit_block(&hex::encode(&raw_block))? {
        None => Ok(Outcome::Accepted {
            hash: solution.hash,
            height: template.height,
            at: solution.at,
            hashes: result.hashes,
        }),
        Some(reason) => Ok(Outcome::Rejected { reason }),
    }
}

/// Whether this template is for the block at exactly the activation height.
///
/// There is no field for this. The template says whether *this* block needs a
/// v2 header; the fork block is the one where that is true and it was not true
/// for the parent, so the parent's header is what has to be looked at.
fn is_fork_block(
    client: &RpcClient,
    template: &BlockTemplate,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !template.header_v2() {
        return Ok(false);
    }

    let parent = client.get_block_header(&template.previous_block_hash)?;
    Ok(!parent.is_header_v2())
}
