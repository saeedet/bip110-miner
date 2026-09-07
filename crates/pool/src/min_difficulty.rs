//! Mining the minimum-difficulty window on testnet4.
//!
//! # The rule
//!
//! testnet4 carries a rule mainnet does not: if a block's timestamp is more
//! than **two block intervals** after its parent's — 20 minutes, given the
//! 10-minute target — that block may be mined at difficulty 1, whatever the
//! chain's real difficulty is.
//!
//! It exists so a test network cannot become unminable when hashpower leaves.
//! Without it, difficulty ratchets up during a burst of mining and the chain
//! stalls forever when that miner goes away.
//!
//! # Why a miner cannot ignore it
//!
//! Because `getblocktemplate` is no use on its own here. It reports the
//! difficulty for the timestamp *it* would pick, which is roughly now — and
//! within 20 minutes of the parent that means the chain's real difficulty,
//! reached by walking back past every minimum-difficulty block in between.
//!
//! Measured on this chain at height 150,615, eight minutes after its parent:
//!
//! ```text
//!   bits 190295cb  ->  difficulty 1,661,387,930
//! ```
//!
//! At the 26 MH/s this project's miner manages, that is some 8,600 years. The
//! same chain at difficulty 1 is **2.7 minutes**. Nothing else about mining
//! here matters as much as which of those two numbers a miner is aiming at.
//!
//! # What this module computes
//!
//! Given the parent's timestamp and the earliest the template will allow, the
//! timestamp that qualifies for minimum difficulty, and the moment a block
//! carrying it becomes legal to submit.
//!
//! # The part worth being deliberate about
//!
//! Consensus also lets a block's timestamp sit up to **two hours ahead** of the
//! present. So when the 20-minute window has not yet opened in real time, a
//! miner may still stamp its block into the future and mine at difficulty 1
//! immediately, rather than waiting.
//!
//! That is standard practice on Bitcoin's testnet4, where essentially every
//! block is minimum difficulty stamped 1201 seconds after an already-future
//! parent. It is not free, though: each such block pushes the chain's clock
//! further ahead of the wall clock, and after about six of them the two-hour
//! allowance is spent and the chain advances only as fast as real time.
//!
//! This module computes the earliest qualifying timestamp and lets the caller
//! decide. [`Window::seconds_ahead_of`] is the honest question — "how far is
//! this block's timestamp ahead of the world's clock?" — and the pool logs the
//! answer on every new tip, so a chain being warped is visible rather than
//! incidental.
//!
//! **None of this applies to mainnet**, which has no minimum-difficulty rule.
//! There, `getblocktemplate` means exactly what it says.

/// Minimum difficulty in compact form.
///
/// testnet4's `powLimit` is `00000000ffff...`, the same value mainnet's
/// difficulty 1 uses, so a hash needs 32 leading zero bits.
pub const MIN_DIFFICULTY_BITS: u32 = 0x1d00_ffff;

/// How far past its parent a block's timestamp must be to qualify: two block
/// intervals of ten minutes each. `nPowTargetSpacing * 2`.
const MIN_DIFFICULTY_GAP: i64 = 20 * 60;

/// How far ahead of the present a block's timestamp may be.
///
/// `MAX_FUTURE_BLOCK_TIME` in `chain.h`. A block beyond it is rejected as
/// `time-too-new` — and rejected *temporarily*: nodes accept it later, once
/// their clocks catch up. So a block stamped too far ahead is not wasted work,
/// merely early.
const MAX_FUTURE_BLOCK_TIME: i64 = 2 * 60 * 60;

/// A plan for mining one minimum-difficulty block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// The timestamp our block must carry.
    ///
    /// The *smallest* qualifying value, deliberately: every second added here
    /// is a second longer before the block may be submitted, and a second more
    /// the chain's clock runs ahead of the world's.
    pub ntime: u32,
    /// Unix time at which a block carrying [`Self::ntime`] stops being
    /// "too far in the future" and can be submitted.
    pub legal_at: i64,
}

impl Window {
    /// How long to wait before a block for this window may be submitted.
    ///
    /// Zero means it can go out immediately. Anything else means the timestamp
    /// is more than two hours ahead, so the block must be **held** rather than
    /// discarded — it is early, not invalid. See [`crate::validate`].
    pub fn seconds_to_wait(&self, now: i64) -> i64 {
        (self.legal_at - now).max(0)
    }

    /// How far ahead of `now` this block's timestamp is.
    ///
    /// Zero means the window opened on its own and no clock is being pushed.
    /// A positive number is the warp, in seconds, and is worth logging.
    pub fn seconds_ahead_of(&self, now: i64) -> i64 {
        (i64::from(self.ntime) - now).max(0)
    }
}

/// Works out the minimum-difficulty window that follows `parent_time`.
///
/// `min_time` is the template's `mintime` — one second past the median of the
/// last eleven blocks, and the earliest timestamp consensus will accept. It
/// matters because the qualifying timestamp is only *usually* the later of the
/// two. On a chain whose clock has been pushed far ahead, the median catches up
/// and can overtake `parent + 20m`; a block stamped before it is rejected as
/// `time-too-old`, having passed every check this module would otherwise make.
pub fn plan(parent_time: u32, min_time: u32) -> Window {
    // Strictly greater than parent + gap, so one second past it.
    let qualifying = i64::from(parent_time) + MIN_DIFFICULTY_GAP + 1;
    let ntime = qualifying.max(i64::from(min_time));

    Window {
        ntime: ntime as u32,
        legal_at: ntime - MAX_FUTURE_BLOCK_TIME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The situation measured on this chain on 2026-09-07.
    ///
    /// Parent at 13:24:04, wall clock at 13:31:44 — seven minutes in, so the
    /// 20-minute window had not opened and the naive template came back at
    /// difficulty 1,661,387,930.
    #[test]
    fn reproduces_the_observed_testnet4_situation() {
        let now = 1_788_787_904; // 13:31:44Z
        let parent_time = 1_788_787_444; // 13:24:04Z
        let min_time = 1_788_781_016; // template mintime, well in the past

        let window = plan(parent_time, min_time);

        assert_eq!(window.ntime, parent_time + 1201, "one second past parent + 20m");
        assert_eq!(
            window.seconds_to_wait(now),
            0,
            "the two-hour allowance covers a 20-minute stamp, so this is \
             submittable immediately — the wait is a choice, not a rule"
        );
        assert_eq!(
            window.seconds_ahead_of(now),
            741,
            "but it does push the chain's clock 12 minutes ahead of the world's"
        );
    }

    /// The ordinary case on a chain nobody is warping: the parent is already
    /// more than 20 minutes old, so the qualifying timestamp is in the past and
    /// nothing is being pushed forward.
    #[test]
    fn an_old_parent_needs_no_warp() {
        let now = 2_000_000_000;
        let window = plan((now - 3600) as u32, 0);

        assert_eq!(window.seconds_to_wait(now), 0);
        assert_eq!(window.seconds_ahead_of(now), 0, "no clock pushing at all");
    }

    /// The chosen timestamp must be the smallest that qualifies. A larger one
    /// is equally valid to consensus but opens the window later and warps the
    /// chain further, which loses the race to anyone who picked the minimum.
    #[test]
    fn picks_the_earliest_qualifying_timestamp() {
        let window = plan(1_000_000, 0);

        assert_eq!(
            i64::from(window.ntime) - 1,
            1_000_000 + MIN_DIFFICULTY_GAP,
            "exactly one second past the gap, not more"
        );
    }

    /// The fix this module has over a naive `parent + 1201`.
    ///
    /// Once a chain has been warped for a while, the median of the last eleven
    /// blocks is itself in the future and can overtake `parent + 20m` — the
    /// parent being merely the most recent block, not the latest-stamped. A
    /// block below `mintime` is rejected as `time-too-old`, and it would have
    /// satisfied every other rule on the way there.
    #[test]
    fn mintime_wins_when_the_median_has_overtaken_the_parent() {
        let parent_time = 1_000_000;
        let min_time = parent_time + 5000; // median already far ahead

        let window = plan(parent_time, min_time);

        assert_eq!(window.ntime, min_time);
        assert!(
            i64::from(window.ntime) > i64::from(parent_time) + MIN_DIFFICULTY_GAP,
            "and it still qualifies for minimum difficulty"
        );
    }

    /// The window opens exactly two hours before the timestamp it carries.
    #[test]
    fn opens_two_hours_before_its_own_timestamp() {
        let window = plan(1_500_000_000, 0);
        assert_eq!(i64::from(window.ntime) - window.legal_at, MAX_FUTURE_BLOCK_TIME);
    }

    /// Boundary: at exactly `legal_at` the block is submittable.
    #[test]
    fn boundary_is_inclusive() {
        let window = plan(1_500_000_000, 0);
        assert_eq!(window.seconds_to_wait(window.legal_at - 1), 1);
        assert_eq!(window.seconds_to_wait(window.legal_at), 0);
    }
}
