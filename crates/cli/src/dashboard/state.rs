//! Everything the dashboard shows, kept up to date from events.
//!
//! The pool and miner report what happens; [`State::apply`] folds each report
//! into the picture on screen. Nothing here draws or touches the terminal, so
//! every rule about what the screen says can be tested on its own.

use std::collections::VecDeque;

use events::{Event, Level, Report};
use node_rpc::Network;

use crate::config::Power;

/// How many lines of history to keep. More than any screen shows.
const RECENT_CAPACITY: usize = 200;

/// How long hashrate samples are kept: the wide screen's half hour.
const SAMPLE_WINDOW: u64 = 30 * 60;

/// What the node looked like when last asked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeStatus {
    /// Whether it answered at all.
    pub running: bool,
    /// Blocks validated.
    pub blocks: u32,
    /// Headers known.
    pub headers: u32,
    /// Whether it says it is still in initial block download.
    pub initial_download: bool,
    /// When its newest block was made, as Unix time.
    pub tip_time: u64,
    /// Connected peers.
    pub peers: usize,
    /// Peers on this chain, where they can be told apart.
    pub chain_peers: Option<usize>,
    /// The running version: `29.4.2`.
    pub version: Option<String>,
    /// Space the node uses, in bytes.
    pub disk_bytes: Option<f64>,
}

impl NodeStatus {
    /// Whether it has caught up with the network.
    pub fn in_sync(&self) -> bool {
        self.running && !self.initial_download && self.headers <= self.blocks + 2
    }
}

/// Whether the pool has work to hand out.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolState {
    /// Started, no job yet.
    Starting,
    /// Waiting for the node, with the reason.
    Waiting(String),
    /// Handing out work.
    Ready,
    /// Stopped handing out work because the node fell behind.
    Paused(String),
}

/// The block being worked on.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// Its height.
    pub height: u32,
    /// Transactions besides the coinbase.
    pub transactions: usize,
    /// What it pays, in satoshis.
    pub reward: u64,
    /// When the pool built it, as Unix time.
    pub since: u64,
}

/// A block this machine found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Its height.
    pub height: u32,
    /// Its hash.
    pub hash: String,
    /// When, as Unix time.
    pub time: u64,
    /// What it pays, in satoshis, if known.
    pub reward: Option<u64>,
    /// The chance of finding it in this session's work, as "1 in N".
    pub odds: Option<f64>,
}

/// One line of history.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// When, as Unix time.
    pub time: u64,
    /// How much it matters.
    pub level: Level,
    /// What happened.
    pub text: String,
}

/// What covers the dashboard, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// Nothing: the dashboard itself.
    None,
    /// The `?` explanations.
    Help,
    /// The block-found celebration.
    Found,
}

/// The lifetime totals, across every session.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lifetime {
    /// Hashes ever.
    pub total_hashes: u64,
    /// Sessions, including this one.
    pub sessions: u64,
    /// Best leading zero bits ever.
    pub best_zero_bits: u32,
}

/// Everything on screen.
#[derive(Debug, Clone)]
pub struct State {
    /// The network being mined.
    pub network: Network,
    /// When this session started, as Unix time.
    pub started: u64,
    /// The time to draw for, as Unix time.
    pub now: u64,
    /// The node, once asked.
    pub node: Option<NodeStatus>,
    /// The pool.
    pub pool: PoolState,
    /// Whether mining is paused from the keyboard.
    pub user_paused: bool,
    /// The power level, or `None` when an exact thread count was given.
    pub power: Option<Power>,
    /// Hashing threads.
    pub threads: usize,
    /// Logical cores.
    pub cores: usize,
    /// The latest hashrate.
    pub rate: f64,
    /// The session's average hashrate.
    pub average: f64,
    /// Recent hashrates, oldest first, as (Unix time, hashes per second).
    pub samples: VecDeque<(u64, f64)>,
    /// Hashes this session.
    pub session_hashes: u64,
    /// The best leading zero bits this session.
    pub best_zero_bits: u32,
    /// The leading zero bits a block needs.
    pub needed_zero_bits: u32,
    /// The current block's difficulty; zero until the first job.
    pub difficulty: f64,
    /// The block being worked on.
    pub job: Option<Job>,
    /// Totals across every session.
    pub lifetime: Option<Lifetime>,
    /// Where rewards go.
    pub address: Option<String>,
    /// History, newest first.
    pub recent: VecDeque<Entry>,
    /// The most recent block found here.
    pub found: Option<Found>,
    /// What covers the dashboard.
    pub overlay: Overlay,
    /// Why the session ended, if something ended it.
    pub fatal: Option<String>,
    /// The height of the last block found here, to recognise its successor.
    last_found_height: Option<u32>,
}

impl State {
    /// A fresh session.
    pub fn new(network: Network, power: Option<Power>, threads: usize, cores: usize, now: u64) -> Self {
        Self {
            network,
            started: now,
            now,
            node: None,
            pool: PoolState::Starting,
            user_paused: false,
            power,
            threads,
            cores,
            rate: 0.0,
            average: 0.0,
            samples: VecDeque::new(),
            session_hashes: 0,
            best_zero_bits: 0,
            needed_zero_bits: 0,
            difficulty: 0.0,
            job: None,
            lifetime: None,
            address: None,
            recent: VecDeque::new(),
            found: None,
            overlay: Overlay::None,
            fatal: None,
            last_found_height: None,
        }
    }

    /// Hashes a block takes on average, at the current difficulty.
    pub fn expected_hashes(&self) -> f64 {
        self.difficulty * 4_294_967_296.0
    }

    /// The chance this session's work had of finding a block, as "1 in N".
    ///
    /// `None` until there is both work done and a difficulty to weigh it by.
    pub fn session_odds(&self) -> Option<f64> {
        (self.session_hashes > 0 && self.difficulty > 0.0)
            .then(|| self.expected_hashes() / self.session_hashes as f64)
    }

    /// Adds a line to the history.
    pub fn note(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        // The same message twice in a row says nothing new: a node that is
        // still catching up says so on every check.
        if self.recent.front().is_some_and(|last| last.text == text) {
            return;
        }
        self.recent.push_front(Entry { time: self.now, level, text });
        self.recent.truncate(RECENT_CAPACITY);
    }

    /// Folds one event into the picture.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Log { level: Level::Info, .. } => {}
            Event::Log { level, text } => {
                if level == Level::Fatal {
                    self.fatal = Some(text.clone());
                }
                self.note(level, text.trim());
            }
            Event::PoolStarted { address, .. } => self.address = Some(address),
            Event::WaitingForNode { reason } => {
                self.note(Level::Warn, format!("waiting for the node: {reason}"));
                self.pool = PoolState::Waiting(reason);
            }
            Event::PoolReady => self.note(Level::Info, "mining started"),
            Event::NewJob { height, transactions, reward, .. } => self.new_job(height, transactions, reward),
            Event::MinimumDifficulty { .. } => {
                self.note(
                    Level::Info,
                    "testnet4's minimum-difficulty window is open: mining at difficulty 1",
                );
            }
            Event::DifficultyChanged { difficulty, .. } => self.difficulty = difficulty,
            Event::Paused { reason } => {
                self.note(Level::Warn, format!("mining paused: {reason}"));
                self.pool = PoolState::Paused(reason);
            }
            Event::Resumed => {
                self.note(Level::Info, "node back in sync — mining resumed");
                self.pool = PoolState::Ready;
            }
            Event::BlockFound { height, hash, .. } => self.block_found(height, hash),
            Event::BlockStale { reason, .. } => self.note(
                Level::Warn,
                format!("a block you solved lost a race to another miner's ({reason}) — that happens"),
            ),
            Event::BlockRejected { reason, .. } => {
                self.note(Level::Fatal, format!("the node REJECTED a block you solved: {reason}"));
            }
            Event::BlockHeld { seconds } => self.note(
                Level::Info,
                format!("holding a solved block for {seconds}s, until its timestamp is allowed"),
            ),
            Event::Hashing { threads, cores } => {
                self.threads = threads;
                self.cores = cores;
            }
            Event::LifetimeLoaded { total_hashes, sessions, best_zero_bits } => {
                self.lifetime = Some(Lifetime { total_hashes, sessions, best_zero_bits });
            }
            Event::JobReceived { difficulty, .. } => self.difficulty = difficulty,
            Event::ShareRejected { reason } => {
                self.note(Level::Warn, format!("the pool rejected a share: {reason}"))
            }
            Event::PoolClosed => self.note(Level::Warn, "the pool closed the connection"),
            Event::Report(report) => self.report(&report),
            Event::Listening { .. }
            | Event::MinerConnected { .. }
            | Event::MinerAuthorized { .. }
            | Event::MinerDisconnected { .. }
            | Event::ShareChecked { .. }
            | Event::Connecting { .. }
            | Event::Subscribed { .. }
            | Event::Authorized { .. }
            | Event::PoolDifficulty { .. }
            | Event::SolutionFound { .. }
            | Event::ShareAccepted => {}
        }
    }

    fn new_job(&mut self, height: u32, transactions: usize, reward: u64) {
        let parent = height.saturating_sub(1);
        if self.job.is_none() {
            self.note(
                Level::Info,
                format!("first job ready: working on block {}", crate::commands::grouped(height.into())),
            );
        } else if self.last_found_height == Some(parent) {
            self.note(Level::Info, "your block is on the chain → new job");
        } else {
            self.note(
                Level::Info,
                format!(
                    "someone else found block {} → new job, nothing lost",
                    crate::commands::grouped(parent.into())
                ),
            );
        }
        self.job = Some(Job { height, transactions, reward, since: self.now });
        self.pool = PoolState::Ready;
    }

    fn block_found(&mut self, height: u32, hash: String) {
        let reward = self.job.as_ref().filter(|job| job.height == height).map(|job| job.reward);
        self.note(Level::Info, format!("★ YOU FOUND BLOCK {} ★", crate::commands::grouped(height.into())));
        self.found = Some(Found { height, hash, time: self.now, reward, odds: self.session_odds() });
        self.last_found_height = Some(height);
        // Regtest finds blocks many times a second; a celebration for each
        // would make the dashboard unusable for the testing it exists for.
        if self.network != Network::Regtest {
            self.overlay = Overlay::Found;
        }
    }

    fn report(&mut self, report: &Report) {
        if report.best_zero_bits > self.best_zero_bits && self.session_hashes > 0 {
            self.note(Level::Info, format!("new personal best: {} zero bits", report.best_zero_bits));
        }
        self.rate = report.recent_rate;
        self.average = report.average_rate;
        self.session_hashes = report.session_hashes;
        self.best_zero_bits = report.best_zero_bits;
        self.needed_zero_bits = report.needed_zero_bits;

        self.samples.push_back((report.unix_time, report.recent_rate));
        while self.samples.front().is_some_and(|(time, _)| *time + SAMPLE_WINDOW < report.unix_time) {
            self.samples.pop_front();
        }

        if let Some(lifetime) = &report.lifetime {
            let sessions = self.lifetime.as_ref().map_or(1, |known| known.sessions);
            self.lifetime = Some(Lifetime {
                total_hashes: lifetime.total_hashes,
                sessions,
                best_zero_bits: lifetime.best_zero_bits,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(Network::Mainnet, Some(Power::Balanced), 6, 8, 1_000)
    }

    fn job(height: u32) -> Event {
        Event::NewJob { height, job_id: "1".into(), transactions: 3, miners: 1, reward: 312_500_000 }
    }

    #[test]
    fn the_first_job_and_later_ones_read_differently() {
        let mut state = state();
        state.apply(job(100));
        state.apply(job(101));
        assert_eq!(state.recent[1].text, "first job ready: working on block 100");
        assert_eq!(state.recent[0].text, "someone else found block 100 → new job, nothing lost");
    }

    #[test]
    fn our_own_block_is_not_credited_to_someone_else() {
        let mut state = state();
        state.apply(job(100));
        state.apply(Event::BlockFound { height: 100, hash: "00ab".into(), peer: "p".into() });
        state.apply(job(101));
        assert_eq!(state.recent[0].text, "your block is on the chain → new job");
        assert_eq!(state.overlay, Overlay::Found);
        assert_eq!(state.found.as_ref().and_then(|found| found.reward), Some(312_500_000));
    }

    #[test]
    fn regtest_blocks_do_not_cover_the_screen() {
        let mut state = State::new(Network::Regtest, None, 2, 8, 1_000);
        state.apply(job(5));
        state.apply(Event::BlockFound { height: 5, hash: "00ab".into(), peer: "p".into() });
        assert_eq!(state.overlay, Overlay::None);
    }

    #[test]
    fn repeated_messages_are_shown_once() {
        let mut state = state();
        for _ in 0..3 {
            state.apply(Event::WaitingForNode { reason: "catching up".into() });
        }
        assert_eq!(state.recent.len(), 1);
    }

    #[test]
    fn odds_need_both_work_and_a_difficulty() {
        let mut state = state();
        assert_eq!(state.session_odds(), None);
        state.difficulty = 1.0;
        state.session_hashes = 4_294_967_296 / 2;
        assert_eq!(state.session_odds(), Some(2.0));
    }

    #[test]
    fn a_fatal_line_is_kept_to_print_after_the_screen_closes() {
        let mut state = state();
        state.apply(Event::Log { level: Level::Fatal, text: "the node went away".into() });
        assert_eq!(state.fatal.as_deref(), Some("the node went away"));
    }
}
