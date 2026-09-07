//! Stratum V1 — the protocol between a mining pool and its miners.
//!
//! Stratum is line-delimited JSON over TCP, and the whole protocol is five
//! methods. A session looks like this:
//!
//! ```text
//! miner -> pool  mining.subscribe                     "what is my extranonce1?"
//! pool  -> miner [[..], extranonce1, extranonce2_size]
//! miner -> pool  mining.authorize  ["worker", "x"]
//! pool  -> miner true
//! pool  -> miner mining.set_difficulty [1.0]           (notification)
//! pool  -> miner mining.notify [job...]                (notification)
//! miner -> pool  mining.submit [worker, job, en2, ntime, nonce]
//! pool  -> miner true
//! ```
//!
//! # Why this fits a chain it predates by fifteen years
//!
//! Stratum V1 was designed around Bitcoin's mining loop, where a miner rolls a
//! 32-bit nonce, and when that runs out rebuilds the coinbase to move the
//! merkle root. Almost every field in `mining.notify` exists to serve that
//! rebuild.
//!
//! This fork removed the need for it. The extranonce lives in the header, so
//! the merkle root never moves. That could have meant a new protocol; instead
//! it means the old one with two of its fields **empty**, which turns out to
//! say exactly what changed. See [`job`].
//!
//! The mapping is not invented here. The node's own source says where these
//! bytes are meant to go:
//!
//! ```text
//!     // These fields get sent to mining machines over Sv1
//!     DataStream ss;
//!     ss << (uint32_t)0;   // Final 3 bytes are part of Sv1 "coinb1"
//!     ss << h2_hash;       // Remainder of Sv1 "coinb1"
//!     ss << m_extranonce;  // Sv1 "extranonce"
//! ```
//!
//! # Shared on purpose
//!
//! This crate is used by both ends. The pool and the miner encode and decode
//! with the *same* functions, so they cannot drift apart on the details that
//! are easy to get wrong — above all byte order, which has no error case and
//! simply produces a miner that never finds anything.
//!
//! Nothing here does I/O. These are types and transformations; the sockets live
//! in the pool and miner crates.

pub mod job;
pub mod message;
pub mod share;

pub use job::{Job, Subscription};
pub use message::{Incoming, Request, Response, StratumError};
pub use share::Share;

/// Method names, so a typo becomes a compile error rather than a silent
/// protocol mismatch.
pub mod method {
    /// Client asks for its extranonce assignment.
    pub const SUBSCRIBE: &str = "mining.subscribe";
    /// Client identifies its worker.
    pub const AUTHORIZE: &str = "mining.authorize";
    /// Client submits a share.
    pub const SUBMIT: &str = "mining.submit";
    /// Server pushes new work.
    pub const NOTIFY: &str = "mining.notify";
    /// Server sets the share difficulty.
    pub const SET_DIFFICULTY: &str = "mining.set_difficulty";
}
