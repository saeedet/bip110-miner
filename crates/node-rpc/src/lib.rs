//! A small JSON-RPC client for a local Bitcoin Knots node running this fork.
//!
//! Scoped to what a solo miner needs: fetch a block template, submit a solved
//! block, and ask the node where the tip is. It authenticates with the cookie
//! file the node writes on startup, so no password is stored anywhere.
//!
//! # What the fork changed here
//!
//! Almost nothing, which is the interesting part. `getblocktemplate` returns
//! the same shape it always did — no new fields for the v2 header's nonces,
//! extranonce, or merge-mining slot, because those are the *miner's* to choose
//! and the node has no opinion about them. The one difference is that the
//! client must declare it understands the `blake2b` rule, or the node refuses
//! to produce a template at all. See [`RpcClient::get_block_template`].
//!
//! The HTTP and base64 layers are written here rather than taken as
//! dependencies — see [`http`] for why that is reasonable for a loopback-only
//! client and unreasonable for anything else.

pub mod auth;
pub mod client;
pub mod http;
pub mod methods;
pub mod network;
pub mod template;

pub use client::{RpcClient, RpcError};
pub use methods::{AddressInfo, BlockHeaderInfo, BlockchainInfo};
pub use network::Network;
pub use template::{BlockTemplate, TemplateTransaction};
