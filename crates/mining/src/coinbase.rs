//! Building the coinbase — the transaction that pays the miner.
//!
//! Every block's first transaction is special. It has no real input, and its
//! outputs create coins that did not previously exist: the block subsidy plus
//! every fee paid by the other transactions in the block. It is the only place
//! where value appears from nothing, and it is the entire economic point of
//! mining.
//!
//! It is also where most homemade miners fail, because four separate consensus
//! rules all land on this one transaction:
//!
//! - **BIP 34** — the `scriptSig` must begin with the block height, encoded as
//!   a minimal Script number. Get this wrong and the block is rejected with
//!   `bad-cb-height`.
//! - **BIP 141** — if SegWit is active, an `OP_RETURN` output must carry the
//!   witness commitment, and the input's witness stack must hold exactly one
//!   32-byte item. Rejections here read `bad-witness-merkle-match` or
//!   `bad-witness-nonce-size`.
//! - **The value cap** — outputs may total no more than subsidy plus fees.
//!   Exceeding it gives `bad-cb-amount`.
//! - **The headline** — this fork's own rule, and the subject of the next
//!   section.
//!
//! # The headline
//!
//! At *exactly* the height BLAKE2b activates, and at no other height, the
//! coinbase `scriptSig` must contain a chain-specific byte string. On mainnet
//! that is the fork's dateline, in the tradition of the genesis block's
//! newspaper headline; on regtest it is whatever `-blake2b_headline` was set
//! to. Miss it and `validation.cpp` rejects the block with `bad-headline`.
//!
//! The check is `std::search` over the whole `scriptSig` — a substring test —
//! so the headline may sit anywhere and needs no particular encoding around
//! it. This builder pushes it as ordinary data after the height, which is what
//! the node's own miner does.
//!
//! # Why the extranonce is still here
//!
//! On Bitcoin, the `scriptSig`'s spare room *is* the extranonce: rolling it
//! changes the coinbase's txid, hence the merkle root, hence a fresh 2³² nonce
//! space. This fork moved a 128-bit extranonce into the header and made that
//! unnecessary — the merkle root need never move again.
//!
//! The field is kept anyway, for two reasons. Before the fork height the
//! header is the classic 80-byte one and the old mechanism is all there is.
//! And after it, an extranonce in the coinbase is still the only way to give
//! two miners disjoint search spaces without them sharing a header field.

use btcb2_primitives::{OutPoint, Transaction, TxIn, TxOut};

use crate::script::{encode_block_height, push_data};
use crate::witness;

/// The minimum size of a coinbase `scriptSig`, in bytes.
pub const MIN_SCRIPT_SIG: usize = 2;
/// The maximum size of a coinbase `scriptSig`, in bytes.
pub const MAX_SCRIPT_SIG: usize = 100;

/// Assembles a coinbase transaction.
#[derive(Debug, Clone)]
pub struct CoinbaseBuilder {
    height: u32,
    value: u64,
    payout_script: Vec<u8>,
    extranonce: Vec<u8>,
    tag: Vec<u8>,
    headline: Option<Vec<u8>>,
    witness_commitment: Option<Vec<u8>>,
}

impl CoinbaseBuilder {
    /// Starts a coinbase paying `value` satoshis to `payout_script` at `height`.
    pub fn new(height: u32, value: u64, payout_script: Vec<u8>) -> Self {
        Self {
            height,
            value,
            payout_script,
            extranonce: Vec::new(),
            tag: Vec::new(),
            headline: None,
            witness_commitment: None,
        }
    }

    /// Sets the extranonce — the bytes varied to refresh the merkle root.
    pub fn extranonce(mut self, extranonce: impl Into<Vec<u8>>) -> Self {
        self.extranonce = extranonce.into();
        self
    }

    /// Sets an arbitrary tag, the "mined by" text pools traditionally embed.
    pub fn tag(mut self, tag: impl Into<Vec<u8>>) -> Self {
        self.tag = tag.into();
        self
    }

    /// Embeds the fork headline. Required at the activation height, and only
    /// there — see the module docs.
    pub fn headline(mut self, headline: impl Into<Vec<u8>>) -> Self {
        self.headline = Some(headline.into());
        self
    }

    /// Adds the SegWit commitment output and the required witness item.
    ///
    /// Pass the script from [`witness::commitment_script`]. Omit this only when
    /// SegWit is inactive, which on any live network it is not.
    pub fn witness_commitment(mut self, script: Vec<u8>) -> Self {
        self.witness_commitment = Some(script);
        self
    }

    /// The `scriptSig` this builder will produce.
    ///
    /// Exposed so a caller can check the length before committing to an
    /// extranonce size — the 100-byte cap is easy to breach once a headline is
    /// in there too.
    pub fn script_sig(&self) -> Vec<u8> {
        let mut script = Vec::new();

        // BIP 34: the height, first, encoded exactly as the node expects.
        // Not a plain push — see `encode_block_height`.
        script.extend_from_slice(&encode_block_height(self.height));

        // Everything after is free-form. All three are pushed rather than
        // appended raw so arbitrary bytes can never be read as opcodes.
        if let Some(headline) = &self.headline {
            push_data(headline, &mut script);
        }
        if !self.extranonce.is_empty() {
            push_data(&self.extranonce, &mut script);
        }
        if !self.tag.is_empty() {
            push_data(&self.tag, &mut script);
        }

        script
    }

    /// Builds the transaction.
    pub fn build(&self) -> Result<Transaction, CoinbaseError> {
        let script_sig = self.script_sig();

        if script_sig.len() < MIN_SCRIPT_SIG || script_sig.len() > MAX_SCRIPT_SIG {
            return Err(CoinbaseError::ScriptSigLength(script_sig.len()));
        }

        // A coinbase carries a witness only when it also carries a commitment.
        // Adding one without the other makes the block invalid either way.
        let witness = match self.witness_commitment {
            Some(_) => vec![witness::RESERVED_VALUE.to_vec()],
            None => Vec::new(),
        };

        let mut outputs = vec![TxOut {
            value: self.value,
            script_pubkey: self.payout_script.clone(),
        }];

        // The commitment output pays zero — it exists only to be committed to.
        if let Some(script) = &self.witness_commitment {
            outputs.push(TxOut {
                value: 0,
                script_pubkey: script.clone(),
            });
        }

        Ok(Transaction {
            // Version 2 enables BIP 68 relative locktimes. Irrelevant for a
            // coinbase, but it is what every modern miner emits.
            version: 2,
            inputs: vec![TxIn {
                // A coinbase spends nothing, so its outpoint is the null one.
                previous_output: OutPoint::NULL,
                script_sig,
                sequence: 0xFFFF_FFFF,
                witness,
            }],
            outputs,
            lock_time: 0,
        })
    }
}

/// Why a coinbase could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoinbaseError {
    /// The `scriptSig` fell outside the consensus range of 2 to 100 bytes.
    ScriptSigLength(usize),
}

impl std::fmt::Display for CoinbaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScriptSigLength(length) => write!(
                f,
                "coinbase scriptSig is {length} bytes, outside the consensus range \
                 of {MIN_SCRIPT_SIG}..={MAX_SCRIPT_SIG} (shrink the extranonce, tag, \
                 or headline)"
            ),
        }
    }
}

impl std::error::Error for CoinbaseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn payout() -> Vec<u8> {
        // A p2wpkh script: OP_0, then a 20-byte push.
        let mut script = vec![0x00, 0x14];
        script.extend_from_slice(&[0xAB; 20]);
        script
    }

    /// The headline has to survive as a contiguous run of bytes, because the
    /// node looks for it with a substring search over the whole scriptSig.
    #[test]
    fn headline_appears_verbatim_in_the_script_sig() {
        let headline = b"btcb2-miner regtest";
        let script = CoinbaseBuilder::new(10, 5_000_000_000, payout())
            .headline(headline.to_vec())
            .extranonce(vec![0x01, 0x02, 0x03, 0x04])
            .script_sig();

        assert!(
            script.windows(headline.len()).any(|w| w == headline),
            "the node searches for the headline as a substring; it was not found"
        );
    }

    /// And it must be absent when not asked for — the rule applies at exactly
    /// one height, so emitting it everywhere would be wrong (harmless, but
    /// wrong, and it eats scarce scriptSig bytes).
    #[test]
    fn headline_is_absent_by_default() {
        let script = CoinbaseBuilder::new(11, 5_000_000_000, payout()).script_sig();
        assert!(!script.windows(5).any(|w| w == b"btcb2"));
    }

    /// A headline plus a large extranonce can breach the 100-byte cap, and it
    /// should fail here rather than at `submitblock`.
    #[test]
    fn oversized_script_sig_is_rejected() {
        let result = CoinbaseBuilder::new(961_640, 0, payout())
            .headline(vec![b'x'; 60])
            .extranonce(vec![0u8; 60])
            .build();

        assert!(matches!(result, Err(CoinbaseError::ScriptSigLength(_))));
    }
}
