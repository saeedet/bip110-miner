//! The nonce search — the loop that actually does the mining.
//!
//! Everything else in this project exists to set up this function. It takes a
//! [`PowMidstate`], tries nonces, and stops when one produces a hash at or
//! below the target.
//!
//! There is no cleverness available. The hash is unpredictable by design, so
//! there is no way to steer toward a solution, no partial progress, and nothing
//! learned from a failed attempt. Mining is a memoryless search: every nonce is
//! an independent trial, which is exactly why stopping and restarting a miner
//! costs nothing.
//!
//! # 128 bits, not 32
//!
//! Bitcoin gives a miner a 32-bit nonce, and 32 bits is nothing — a modern ASIC
//! exhausts it in well under a second, which is why Bitcoin mining has to keep
//! rebuilding the coinbase to move the merkle root. That machinery is the
//! reason Stratum looks the way it does.
//!
//! This fork's header has four grindable 32-bit words instead, and none of them
//! touches the merkle root:
//!
//! | Word | What it is |
//! |---|---|
//! | `nonce` | the classic one |
//! | `nonce2`, `nonce3` | added by the v2 header, purely for grinding |
//! | `time_offset` | see below |
//!
//! `time_offset` earns its place by an accident of the design. It is added to
//! the wire timestamp to give the block's real time — but *only* when the
//! `UseTimeOffset` flag is set. With the flag clear the field is consensus-inert:
//! the node reads the timestamp straight off the wire and never looks at it,
//! and nothing in `validation.cpp` constrains it. Yet stage 4 of the proof of
//! work hashes it. So it is a fourth free nonce word, and this search treats
//! it as one.
//!
//! Together that is 2¹²⁸ hashes before the extranonce has to move — which, at
//! any hashrate that has ever existed, is never.
//!
//! All of that applies from the activation height upward. Below it the header
//! is Bitcoin's 80-byte one, hashed with SHA-256d, and there is exactly one
//! nonce word again. [`PowMidstate::nonce_words`] is what keeps the two
//! straight; see its docs for why getting it wrong hangs rather than fails.
//!
//! # The midstate is given, not discovered
//!
//! Bitcoin's midstate optimisation had to be *noticed*: someone had to see that
//! the header's first 64 bytes never change during a sweep and that SHA-256's
//! first compression could therefore be hoisted out of the loop.
//!
//! Here the split is stated by the design. The proof of work folds every
//! consensus field into a digest before any nonce is involved, precisely so
//! that mining hardware never sees a version bit or a timestamp and cannot be
//! stranded by a change to either. [`PowMidstate`] is that boundary, and
//! hoisting it out of the loop is not an optimisation so much as using the
//! interface as intended.

use bip110_primitives::{Hash256, PowMidstate, Target};

/// A point in the 128-bit nonce space.
///
/// Four 32-bit words, ordered least-significant first, so incrementing sweeps
/// `nonce` through all 2³² values before disturbing `nonce2`. That ordering
/// matters: `nonce` is the word a Stratum client would roll, so keeping it
/// innermost makes this search's traversal the same one the wire protocol
/// assumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NonceSpace {
    /// The classic nonce. Rolled first.
    pub nonce: u32,
    /// The v2 header's second nonce word.
    pub nonce2: u32,
    /// The v2 header's third nonce word.
    pub nonce3: u32,
    /// The time offset, free to grind while `UseTimeOffset` is clear.
    pub time_offset: u32,
}

impl NonceSpace {
    /// Advances to the next point, carrying through the first `words` words.
    ///
    /// `words` comes from [`PowMidstate::nonce_words`]: four after the fork,
    /// **one** before it. That is not a tuning knob. On a pre-fork header the
    /// other three words are not serialised, so rolling them would recompute
    /// one hash forever — a search that never ends and never progresses.
    ///
    /// Zeroes the words beyond the first `words`.
    ///
    /// A caller that seeds a starting point does not necessarily know which
    /// header it will be handed — the fork boundary is a property of the
    /// height, not of the miner. Clamping here means a seed aimed at a word
    /// that does not exist becomes zero rather than a value written into a
    /// header field that is never serialised.
    pub fn truncate(&mut self, words: u32) {
        if words < 4 {
            self.time_offset = 0;
        }
        if words < 3 {
            self.nonce3 = 0;
        }
        if words < 2 {
            self.nonce2 = 0;
        }
    }

    /// Wraps silently at the top of whatever space it is given.
    pub fn advance(&mut self, words: u32) {
        // The overwhelmingly common case is a plain increment of one word; a
        // carry happens once every 2^32 hashes.
        let (nonce, carry) = self.nonce.overflowing_add(1);
        self.nonce = nonce;
        if !carry || words < 2 {
            return;
        }

        let (nonce2, carry) = self.nonce2.overflowing_add(1);
        self.nonce2 = nonce2;
        if !carry || words < 3 {
            return;
        }

        let (nonce3, carry) = self.nonce3.overflowing_add(1);
        self.nonce3 = nonce3;
        if carry && words >= 4 {
            self.time_offset = self.time_offset.wrapping_add(1);
        }
    }
}

/// What a search found.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The winning nonces, if the target was met.
    pub solution: Option<Solution>,
    /// How many hashes were computed. Used for hashrate reporting.
    pub hashes: u64,
    /// Where the search stopped, so the next call can pick up from here.
    pub next: NonceSpace,
    /// The lowest hash seen, and the nonces that produced it.
    ///
    /// This is the "how close did I get" figure. It is not consensus-relevant
    /// in solo mining — a near miss is worth precisely nothing, and says
    /// nothing about the next hash — but it is the only feedback a solo miner
    /// ever gets.
    pub best: Hash256,
    /// The point that produced [`Self::best`].
    pub best_at: NonceSpace,
}

/// A set of nonces that meets the target.
#[derive(Debug, Clone, Copy)]
pub struct Solution {
    /// The point in the nonce space that solved it.
    pub at: NonceSpace,
    /// The resulting proof-of-work hash.
    pub hash: Hash256,
}

/// Tries `attempts` consecutive points from `start`, stopping early on a
/// solution.
///
/// Bounded by a count rather than run to exhaustion because the space is 2¹²⁸
/// wide: a caller has to be able to come up for air, check whether the tip
/// moved, and report a hashrate. The returned [`SearchResult::next`] is where
/// to resume.
///
/// On a pre-fork header the space is only 2³² wide, and `attempts` is capped
/// at that so a large budget cannot silently re-search ground it has covered.
pub fn search(
    midstate: &PowMidstate,
    target: &Target,
    start: NonceSpace,
    attempts: u64,
) -> SearchResult {
    let words = midstate.nonce_words();

    // A pre-fork header exposes 2^32 points and no more, so an attempt budget
    // above that would hash the same values a second time and report the work
    // as if it were new.
    let attempts = if words == 1 { attempts.min(1 << 32) } else { attempts };

    let mut start = start;
    start.truncate(words);

    let mut at = start;
    let mut hashes = 0u64;
    let mut best = Hash256::from_internal_bytes([0xFF; 32]);
    let mut best_at = start;

    while hashes < attempts {
        let hash = midstate.hash(at.nonce, at.nonce2, at.nonce3, at.time_offset);
        hashes += 1;

        if hash.is_below(&best) {
            best = hash;
            best_at = at;
        }

        if target.is_met_by(&hash) {
            let mut next = at;
            next.advance(words);
            return SearchResult {
                solution: Some(Solution { at, hash }),
                hashes,
                next,
                best: hash,
                best_at: at,
            };
        }

        at.advance(words);
    }

    SearchResult {
        solution: None,
        hashes,
        next: at,
        best,
        best_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_words_carry_in_order() {
        let mut at = NonceSpace {
            nonce: u32::MAX,
            ..Default::default()
        };
        at.advance(4);
        assert_eq!(
            at,
            NonceSpace {
                nonce: 0,
                nonce2: 1,
                ..Default::default()
            }
        );

        // A carry that ripples the whole way across.
        let mut at = NonceSpace {
            nonce: u32::MAX,
            nonce2: u32::MAX,
            nonce3: u32::MAX,
            time_offset: 7,
        };
        at.advance(4);
        assert_eq!(at, NonceSpace { nonce: 0, nonce2: 0, nonce3: 0, time_offset: 8 });
    }

    #[test]
    fn truncation_clears_words_that_do_not_exist() {
        let mut at = NonceSpace { nonce: 5, nonce2: 9, nonce3: 11, time_offset: 13 };
        at.truncate(1);
        assert_eq!(at, NonceSpace { nonce: 5, ..Default::default() });

        let mut at = NonceSpace { nonce: 5, nonce2: 9, nonce3: 11, time_offset: 13 };
        at.truncate(4);
        assert_eq!(at, NonceSpace { nonce: 5, nonce2: 9, nonce3: 11, time_offset: 13 });
    }

    /// A pre-fork header has one word, and a carry out of it must go nowhere.
    /// If it leaked into `nonce2` the search would still terminate but every
    /// hash after the wrap would be a repeat, silently.
    #[test]
    fn a_single_word_space_wraps_in_place() {
        let mut at = NonceSpace { nonce: u32::MAX, nonce2: 3, nonce3: 4, time_offset: 5 };
        at.advance(1);
        assert_eq!(at, NonceSpace { nonce: 0, nonce2: 3, nonce3: 4, time_offset: 5 });
    }

    /// The common case must not disturb the other three words.
    #[test]
    fn ordinary_increments_touch_only_the_nonce() {
        let mut at = NonceSpace { nonce: 5, nonce2: 9, nonce3: 11, time_offset: 13 };
        at.advance(4);
        assert_eq!(at, NonceSpace { nonce: 6, nonce2: 9, nonce3: 11, time_offset: 13 });
    }
}
