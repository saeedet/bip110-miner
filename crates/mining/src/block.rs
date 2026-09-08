//! Assembling and serialising a complete block.
//!
//! A block on the wire is startlingly simple, and the fork did not change it:
//!
//! ```text
//!   <header> <CompactSize transaction count> <transaction>...
//! ```
//!
//! The header is 80 bytes before the fork and 164 after. Everything downstream
//! of it is byte-for-byte what Bitcoin does.
//!
//! # The count is written twice
//!
//! A v2 header commits to the transaction count in its own `txcount` field,
//! *and* the body still carries the CompactSize count it always did. Both are
//! present; the node checks they agree:
//!
//! ```text
//!   if (block.m_txcount != (block.m_header_v2 ? block.vtx.size() : 0))
//!       ... "bad-txnlist-size"
//! ```
//!
//! The redundancy is the point. It closes CVE-2017-12842, where a 64-byte
//! transaction could be passed off as a pair of merkle nodes and an SPV client
//! fooled into accepting a payment that was never in the block. With the count
//! signed by the proof of work, an attacker cannot restate the tree's shape.
//!
//! Note the `? :` — on a v1 header the field must be **zero**, not the real
//! count. [`BlockBuilder::header`] handles that so a caller cannot get it
//! backwards.
//!
//! Transactions are serialised **with** their witnesses here, unlike in the
//! merkle tree, which uses witness-free txids. Both are needed and they are not
//! interchangeable.

use bip110_primitives::{BlockHeader, Hash256, Transaction, merkle, varint};

/// A block under construction: a coinbase we built plus transactions the node
/// handed us as raw bytes.
///
/// The other transactions stay as bytes rather than being parsed into
/// [`Transaction`] values. There is nothing to gain from round-tripping them —
/// the node already validated them, and re-serialising introduces a chance of
/// producing something subtly different from what was validated.
#[derive(Debug, Clone)]
pub struct BlockBuilder {
    /// The coinbase transaction.
    pub coinbase: Transaction,
    /// The remaining transactions, serialised, in block order.
    pub transactions: Vec<Vec<u8>>,
    /// Their txids, in the same order, for the merkle tree.
    pub txids: Vec<Hash256>,
}

impl BlockBuilder {
    /// Creates a builder from a coinbase and the node's transaction list.
    pub fn new(
        coinbase: Transaction,
        transactions: Vec<Vec<u8>>,
        txids: Vec<Hash256>,
    ) -> Result<Self, BlockError> {
        if transactions.len() != txids.len() {
            return Err(BlockError::CountMismatch {
                transactions: transactions.len(),
                txids: txids.len(),
            });
        }

        Ok(Self {
            coinbase,
            transactions,
            txids,
        })
    }

    /// The merkle root over the coinbase and every other transaction.
    ///
    /// Uses **txids**, not wtxids — the witness tree is a separate structure
    /// committed to inside the coinbase. See [`crate::witness`].
    ///
    /// Still SHA-256d, and necessarily so: the fork replaced the header hash,
    /// not the transaction ids underneath it. Changing those would have
    /// rewritten every txid in the history the two chains share.
    pub fn merkle_root(&self) -> Hash256 {
        let mut leaves = Vec::with_capacity(self.txids.len() + 1);
        leaves.push(self.coinbase.txid());
        leaves.extend_from_slice(&self.txids);

        merkle::merkle_root(&leaves).expect("the coinbase is always present")
    }

    /// How many transactions the block contains, counting the coinbase.
    pub fn transaction_count(&self) -> usize {
        self.transactions.len() + 1
    }

    /// Fills in the header fields this builder is responsible for.
    ///
    /// Takes a header carrying the template's values — version, previous
    /// block, time, bits — and sets the three that depend on the block's
    /// contents: the merkle root, the height, and the transaction count.
    ///
    /// `height` matters only for a v2 header, where it is committed directly.
    /// On v1 it lives in the coinbase alone (BIP 34) and the header field must
    /// read zero, which is what the node's `AreHeaderV2FieldsNull` check
    /// enforces.
    pub fn header(&self, template: &BlockHeader, height: u32) -> Result<BlockHeader, BlockError> {
        let count = self.transaction_count();

        let mut header = *template;
        header.merkle_root = self.merkle_root();

        if header.header_v2 {
            // A u16 caps the count at 65,535. The block weight limit puts the
            // real ceiling far below that — a minimal transaction is about 60
            // weight units short of 250, so ~16,000 is the practical maximum —
            // but the cast has to be checked rather than assumed, because a
            // silent truncation here produces `bad-txnlist-size` at submission
            // with nothing to point at.
            header.txcount = u16::try_from(count).map_err(|_| BlockError::TooManyTransactions(count))?;
            header.height = height;
        } else {
            header.txcount = 0;
            header.height = 0;
        }

        Ok(header)
    }

    /// Serialises the complete block for `submitblock`.
    pub fn serialize(&self, header: &BlockHeader) -> Vec<u8> {
        let mut block = Vec::new();

        block.extend_from_slice(&header.serialize());
        varint::encode(self.transaction_count() as u64, &mut block);

        // The coinbase, with its witness — the reserved value lives there.
        block.extend_from_slice(&self.coinbase.serialize());

        for transaction in &self.transactions {
            block.extend_from_slice(transaction);
        }

        block
    }
}

/// Why a block could not be assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// The transaction and txid lists were different lengths, which means the
    /// merkle root would not describe the block's actual contents.
    CountMismatch {
        /// How many raw transactions were given.
        transactions: usize,
        /// How many txids were given.
        txids: usize,
    },
    /// More transactions than the header's `u16` count can express.
    TooManyTransactions(usize),
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CountMismatch { transactions, txids } => write!(
                f,
                "{transactions} transactions but {txids} txids — \
                 the merkle root would not match the block"
            ),
            Self::TooManyTransactions(count) => write!(
                f,
                "{count} transactions will not fit the header's 16-bit txcount field"
            ),
        }
    }
}

impl std::error::Error for BlockError {}
