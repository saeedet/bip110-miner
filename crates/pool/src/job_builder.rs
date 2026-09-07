//! Turning a block template into a Stratum job.
//!
//! # What this module does not have to do
//!
//! On Bitcoin this is the fiddliest part of a pool. The extranonce lives inside
//! the coinbase transaction, so the pool must serialise the coinbase, find
//! exactly where the extranonce landed, split it in two, send both halves, and
//! also send a merkle branch so the miner can recompute the root after
//! splicing. The sibling project does all of that, and guards it with a
//! sentinel value and a runtime cross-check, because getting the offset wrong
//! makes every share on the pool unusable and does so silently.
//!
//! None of it is here. This fork put the extranonce in the header, so the
//! coinbase is built once per job and never touched again. The merkle root is
//! computed once. There is no split, no branch, no sentinel, and nothing to
//! cross-check, because there is no offset to get wrong.
//!
//! That absence is the clearest single measure of what the hardfork bought.

use btcb2_primitives::{BlockHeader, Hash256, PowMidstate, Target, hex};
use mining::{BlockBuilder, CoinbaseBuilder, witness};
use node_rpc::BlockTemplate;
use stratum::Job;

/// A job, plus everything needed to rebuild a real block from a share.
///
/// The miner receives only [`Self::job`]. The rest stays here, because the
/// miner has no use for it and sending it would mean sending every transaction
/// in the block to every device.
pub struct ActiveJob {
    /// What gets sent to miners.
    pub job: Job,
    /// The header with everything filled in except the nonces and extranonce.
    ///
    /// Fixed for the life of the job. On Bitcoin the merkle root inside this
    /// would move on every extranonce increment; here it cannot.
    pub header: BlockHeader,
    /// The coinbase and the template's transactions, ready to serialise.
    pub block: BlockBuilder,
    /// The height this block would have.
    pub height: u32,
    /// The target a hash must meet to be a real block.
    pub network_target: Target,
}

/// Builds a job from a template.
pub fn build(
    job_id: String,
    template: &BlockTemplate,
    payout_script: &[u8],
    headline: Option<&[u8]>,
    clean_jobs: bool,
) -> Result<ActiveJob, BuildError> {
    let field = |error: String| BuildError::Template(error);

    let wtxids: Vec<Hash256> = template
        .transactions
        .iter()
        .map(|tx| tx.wtxid())
        .collect::<Result<_, _>>()
        .map_err(|error| field(error.to_string()))?;

    let witness_commitment = witness::commitment_script(&wtxids);

    // The node tells us what it expects. Deriving it ourselves and then
    // checking is how we know our derivation is right; disagreeing means one of
    // us is wrong and the block would be rejected either way.
    if let Some(expected) = &template.default_witness_commitment
        && &hex::encode(&witness_commitment) != expected
    {
        return Err(BuildError::WitnessCommitmentMismatch);
    }

    let mut coinbase = CoinbaseBuilder::new(
        template.height,
        template.coinbase_value,
        payout_script.to_vec(),
    )
    .tag(b"btcb2-miner".to_vec())
    .witness_commitment(witness_commitment);

    if let Some(headline) = headline {
        coinbase = coinbase.headline(headline.to_vec());
    }

    let coinbase = coinbase.build().map_err(BuildError::Coinbase)?;

    let txids: Vec<Hash256> = template
        .transactions
        .iter()
        .map(|tx| tx.txid())
        .collect::<Result<_, _>>()
        .map_err(|error| field(error.to_string()))?;

    let transactions: Vec<Vec<u8>> = template
        .transactions
        .iter()
        .map(|tx| tx.raw())
        .collect::<Result<_, _>>()
        .map_err(|error| field(error.to_string()))?;

    let block = BlockBuilder::new(coinbase, transactions, txids)
        .map_err(|error| BuildError::Block(error.to_string()))?;

    let from_template = BlockHeader {
        header_v2: template.header_v2(),
        version: template.base_version(),
        prev_block: template.previous_block().map_err(|e| field(e.to_string()))?,
        merkle_root: Hash256::ZERO,
        time_on_wire: u32::try_from(template.current_time).map_err(|_| BuildError::TimeOverflow)?,
        bits: template.compact_bits().map_err(|e| field(e.to_string()))?,
        nonce: 0,
        nonce2: 0,
        nonce3: 0,
        extranonce: [0u8; 16],
        // Left clear, which is what makes `time_offset` a free nonce word for
        // every miner on this pool. See `stratum::share`.
        time_offset: 0,
        txcount: 0,
        flags: 0,
        // A null XOR key: no masking, so a miner can tell a block from a share.
        // A pool with untrusted miners would set one and keep it secret; this
        // pool's only miner is its operator.
        xor_key_mask_clear_bits: 0,
        xor_key: [0u8; 16],
        height: 0,
        mm_rhs: Hash256::ZERO,
    };

    let header = block
        .header(&from_template, template.height)
        .map_err(|error| BuildError::Block(error.to_string()))?;

    // Stages 0 to 3 of the proof of work, computed from the real header. What
    // the miner receives is a projection of this — see `stratum::job`.
    let PowMidstate::Blake2b(midstate) = PowMidstate::new(&header) else {
        return Err(BuildError::PreForkTemplate(template.height));
    };

    Ok(ActiveJob {
        job: Job {
            job_id,
            prev_hidden: midstate.hidden_prev_block(),
            consensus_digest: *midstate.consensus_digest(),
            flags: header.flags,
            bits: header.bits,
            clean_jobs,
        },
        header,
        block,
        height: template.height,
        network_target: template.target().map_err(|e| field(e.to_string()))?,
    })
}

/// Why a job could not be built.
#[derive(Debug)]
pub enum BuildError {
    /// A field in the template could not be interpreted.
    Template(String),
    /// The coinbase could not be constructed.
    Coinbase(mining::coinbase::CoinbaseError),
    /// The block could not be assembled.
    Block(String),
    /// Our witness commitment disagreed with the node's.
    WitnessCommitmentMismatch,
    /// The template's timestamp did not fit in the header's 32 bits.
    TimeOverflow,
    /// The template asks for a pre-fork header.
    ///
    /// Only reachable on a regtest chain below its activation height. Stratum
    /// here describes the BLAKE2b pipeline; a v1 block does not use it, and
    /// there is no honest job to send for one.
    PreForkTemplate(u32),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Template(message) => write!(f, "bad block template: {message}"),
            Self::Coinbase(source) => write!(f, "cannot build coinbase: {source}"),
            Self::Block(message) => write!(f, "cannot assemble block: {message}"),
            Self::WitnessCommitmentMismatch => write!(
                f,
                "our witness commitment disagrees with the node's — \
                 a block built on it would be rejected"
            ),
            Self::TimeOverflow => write!(f, "template timestamp does not fit in a u32"),
            Self::PreForkTemplate(height) => write!(
                f,
                "height {height} is below the BLAKE2b activation height, so this template \
                 wants an 80-byte header — mine it with regtest-miner instead, which \
                 handles both regimes"
            ),
        }
    }
}

impl std::error::Error for BuildError {}
