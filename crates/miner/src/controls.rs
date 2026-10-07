//! The miner's knobs, turnable while it runs.
//!
//! A standalone miner is set up once from its arguments and then left alone.
//! A dashboard needs more: stop cleanly when you press `q`, pause on `p`,
//! change how many cores it uses on `+` and `-` — all without reconnecting to
//! the pool or losing the job in hand.
//!
//! Each knob is a single atomic, read by the hashing threads between batches.
//! That is the whole mechanism: no locks on the hot path, and a change takes
//! effect within one batch, a few tens of milliseconds.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Live controls for a running miner.
#[derive(Debug)]
pub struct Controls {
    stop: AtomicBool,
    paused: AtomicBool,
    threads: AtomicUsize,
}

impl Controls {
    /// Starts unpaused, hashing on `threads` threads.
    pub fn new(threads: usize) -> Self {
        Self {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            threads: AtomicUsize::new(threads.max(1)),
        }
    }

    /// Asks the miner to finish: threads stop at the next batch, the lifetime
    /// totals are saved, and [`crate::run`] returns.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Whether a stop has been asked for.
    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Pauses or resumes hashing. The connection and the job stay as they are.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Whether hashing is paused.
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Changes how many threads hash. Never below one; threads beyond the
    /// number the miner started with stay idle.
    pub fn set_threads(&self, threads: usize) {
        self.threads.store(threads.max(1), Ordering::Relaxed);
    }

    /// How many threads should be hashing.
    pub fn threads(&self) -> usize {
        self.threads.load(Ordering::Relaxed)
    }

    /// Whether the thread at `index` should be hashing right now.
    pub fn should_hash(&self, index: usize) -> bool {
        !self.paused() && index < self.threads()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_n_threads_hash() {
        let controls = Controls::new(3);
        assert!(controls.should_hash(0));
        assert!(controls.should_hash(2));
        assert!(!controls.should_hash(3), "thread 3 of a 3-thread setting is idle");

        controls.set_threads(1);
        assert!(!controls.should_hash(1));
    }

    #[test]
    fn pausing_idles_every_thread() {
        let controls = Controls::new(8);
        controls.set_paused(true);
        assert!((0..8).all(|i| !controls.should_hash(i)));
        controls.set_paused(false);
        assert!(controls.should_hash(0));
    }

    /// Zero threads would be a pause with no way to say so; the floor is one.
    #[test]
    fn never_drops_below_one_thread() {
        let controls = Controls::new(0);
        assert_eq!(controls.threads(), 1);
        controls.set_threads(0);
        assert_eq!(controls.threads(), 1);
    }
}
