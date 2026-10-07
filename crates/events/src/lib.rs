//! What the pool and the miner have to say, as data.
//!
//! Both used to `println!` from wherever something happened. That is fine for a
//! terminal log and useless to anything else: a dashboard cannot redraw a
//! hashrate it only ever saw as part of a sentence.
//!
//! So they now *emit* [`Event`]s into a [`Sink`], and the sink decides what to
//! do with them. [`Plain`] prints exactly the lines the programs always
//! printed, so nothing changes for anyone reading a log. A dashboard uses a
//! [`Channel`] and draws the same facts however it likes.
//!
//! # Structured, or just a line?
//!
//! An event carries fields when something downstream needs the values — a
//! hashrate to plot, a block height to show, a best hash to compare. Messages
//! that are only ever read, such as "cannot parse from pool", travel as
//! [`Event::Log`] with a [`Level`], which is enough to colour them and file
//! them under "recent".

mod plain;

pub use plain::Plain;

use std::sync::Mutex;
use std::sync::mpsc::Sender;

/// How much attention a [`Event::Log`] line deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Ordinary progress.
    Info,
    /// Something went wrong but the program carries on.
    Warn,
    /// The program is about to stop because of this.
    Fatal,
}

/// Something the pool or the miner wants known.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A line with nothing structured in it.
    Log {
        /// How serious it is.
        level: Level,
        /// The message, as it would be printed.
        text: String,
    },

    // --- pool ---------------------------------------------------------------
    /// The pool has a node it trusts and an address to pay.
    PoolStarted {
        /// The payout address, as typed.
        address: String,
        /// The payout script, as hex.
        payout_script: String,
        /// Network name.
        network: String,
        /// Where the pool listens for miners.
        listen: String,
        /// The node's height at startup.
        height: u32,
        /// The node's peer count at startup.
        peers: u32,
    },
    /// The Stratum port is open.
    Listening {
        /// The address it is bound to.
        listen: String,
    },
    /// The node is not fit to mine on yet, and the pool is waiting for it.
    WaitingForNode {
        /// Why, in plain words.
        reason: String,
    },
    /// The pool built new work.
    NewJob {
        /// The height of the block being built.
        height: u32,
        /// The Stratum job id.
        job_id: String,
        /// Transactions in the block besides the coinbase.
        transactions: usize,
        /// Miners connected when it went out.
        miners: usize,
    },
    /// The job was built for testnet4's minimum-difficulty window.
    MinimumDifficulty {
        /// The difficulty mined at.
        difficulty: f64,
        /// How far ahead of the wall clock the block's timestamp is, in seconds.
        seconds_ahead: i64,
    },
    /// The target changed without the tip moving.
    DifficultyChanged {
        /// The height of the block being built.
        height: u32,
        /// The Stratum job id.
        job_id: String,
        /// The new difficulty.
        difficulty: f64,
        /// Miners connected.
        miners: usize,
    },
    /// The first job is built; miners can do useful work.
    PoolReady,
    /// Mining is paused because the node stopped being fit to mine on.
    Paused {
        /// Why, in plain words.
        reason: String,
    },
    /// The node recovered and mining continues.
    Resumed,
    /// A miner opened a connection.
    MinerConnected {
        /// Its address.
        peer: String,
        /// The extranonce prefix it was given, as hex.
        extranonce1: String,
    },
    /// A miner identified itself.
    MinerAuthorized {
        /// Its address.
        peer: String,
        /// The worker name it gave.
        worker: String,
    },
    /// A miner went away.
    MinerDisconnected {
        /// Its address.
        peer: String,
    },
    /// A share met the network target and the node accepted the block.
    BlockFound {
        /// Its height.
        height: u32,
        /// Its hash, in display order.
        hash: String,
        /// Which miner found it.
        peer: String,
    },
    /// A valid block lost a race — `inconclusive` or `duplicate`.
    BlockStale {
        /// Which miner found it.
        peer: String,
        /// Its hash.
        hash: String,
        /// The node's reason.
        reason: String,
    },
    /// The node refused a block whose proof of work was real: always our bug.
    BlockRejected {
        /// Which miner found it.
        peer: String,
        /// Its hash.
        hash: String,
        /// The node's reason.
        reason: String,
    },
    /// A share checked out but was not a block.
    ShareChecked {
        /// Which miner sent it.
        peer: String,
        /// Its leading zero bits.
        zero_bits: u32,
        /// Its hash.
        hash: String,
    },
    /// A solved block carries a future timestamp and is being held back.
    BlockHeld {
        /// How long until it may be submitted.
        seconds: i64,
    },

    // --- miner --------------------------------------------------------------
    /// The miner is dialling the pool.
    Connecting {
        /// The pool's address.
        pool: String,
    },
    /// The pool assigned an extranonce.
    Subscribed {
        /// The pool's half, as hex.
        extranonce1: String,
        /// How many bytes the miner supplies.
        extranonce2_size: usize,
    },
    /// The pool accepted the worker name.
    Authorized {
        /// The worker name.
        worker: String,
    },
    /// Hashing threads are running.
    Hashing {
        /// Threads in use.
        threads: usize,
        /// Logical cores available.
        cores: usize,
    },
    /// Lifetime totals loaded at startup.
    LifetimeLoaded {
        /// Hashes across every session.
        total_hashes: u64,
        /// Sessions, including this one.
        sessions: u64,
        /// Best leading zero bits ever.
        best_zero_bits: u32,
    },
    /// The pool announced a share difficulty.
    PoolDifficulty {
        /// The difficulty.
        difficulty: f64,
    },
    /// New work arrived from the pool.
    JobReceived {
        /// The Stratum job id.
        job_id: String,
        /// Its difficulty.
        difficulty: f64,
        /// Whether earlier work was discarded.
        clean: bool,
        /// The block it builds on, where the protocol reveals it.
        ///
        /// Bitcoin's Stratum sends the previous block hash; BIP-110's hides it
        /// from miners by design, so there it is `None`.
        prev_hash: Option<String>,
    },
    /// A hash met the target.
    SolutionFound {
        /// The hash.
        hash: String,
        /// Its leading zero bits.
        zero_bits: u32,
    },
    /// The pool accepted a share.
    ShareAccepted,
    /// The pool rejected a share.
    ShareRejected {
        /// The pool's reason.
        reason: String,
    },
    /// The pool hung up.
    PoolClosed,
    /// The periodic status report.
    Report(Report),
}

/// The miner's status, every few seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Unix time it was taken.
    pub unix_time: u64,
    /// Hashes per second over the last interval.
    pub recent_rate: f64,
    /// Hashes per second since the session started.
    pub average_rate: f64,
    /// Hashes this session.
    pub session_hashes: u64,
    /// The best hash this session, in display order.
    pub best_hash: String,
    /// Its leading zero bits.
    pub best_zero_bits: u32,
    /// The leading zero bits a block needs.
    pub needed_zero_bits: u32,
    /// Lifetime totals; absent until a job with a usable target has arrived.
    pub lifetime: Option<LifetimeReport>,
}

/// The lifetime half of a [`Report`].
#[derive(Debug, Clone, PartialEq)]
pub struct LifetimeReport {
    /// Hashes across every session.
    pub total_hashes: u64,
    /// Best leading zero bits ever.
    pub best_zero_bits: u32,
    /// The work done, as "about 1 in N" of a block.
    pub odds_denominator: f64,
}

/// Somewhere events go.
///
/// `Send + Sync` because the pool and miner emit from many threads at once.
pub trait Sink: Send + Sync {
    /// Accepts one event. Must not block for long: callers include the
    /// hashing threads.
    fn emit(&self, event: Event);
}

/// Forwards events to a channel, for a dashboard to read.
pub struct Channel(Mutex<Sender<Event>>);

impl Channel {
    /// Wraps the sending half of a channel.
    pub fn new(sender: Sender<Event>) -> Self {
        Self(Mutex::new(sender))
    }
}

impl Sink for Channel {
    fn emit(&self, event: Event) {
        // A closed receiver means the reader has gone away, at which point
        // nobody is left to tell. Dropping the event is the only sensible move.
        if let Ok(sender) = self.0.lock() {
            let _ = sender.send(event);
        }
    }
}

/// Formats a count with an SI-style suffix: `3.04T`, `215.90G`.
///
/// These numbers reach the quadrillions over a few sessions, and raw digits at
/// that scale convey nothing.
pub fn si(count: u64) -> String {
    const UNITS: [(f64, &str); 5] = [(1e18, "E"), (1e15, "P"), (1e12, "T"), (1e9, "G"), (1e6, "M")];
    let value = count as f64;
    for (scale, suffix) in UNITS {
        if value >= scale {
            return format!("{:.2}{suffix}", value / scale);
        }
    }
    format!("{count}")
}

/// Formats a Unix timestamp as `HH:MM:SS` in UTC.
///
/// UTC rather than local time, deliberately: a log line that cannot be matched
/// against a block's timestamp is less useful than one that can, and block
/// timestamps are UTC.
pub fn clock(unix: u64) -> String {
    let seconds_today = unix % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds_today / 3600,
        (seconds_today % 3600) / 60,
        seconds_today % 60,
    )
}
