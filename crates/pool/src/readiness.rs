//! Deciding whether a node is actually fit to mine on.
//!
//! # Why `initialblockdownload` is not enough
//!
//! Bitcoin Core's `IsInitialBlockDownload()` **latches**, and Knots inherits it. Once it goes false it
//! never returns true again for the life of that process:
//!
//! ```text
//! if (m_cached_finished_ibd.load(...)) return false;   // never re-evaluated
//! ```
//!
//! So a node that syncs to the tip and then loses every peer — a laptop that
//! slept, changed networks, or landed behind a firewall — reports
//! `initialblockdownload: false` **for ever**, with a tip frozen hours or days
//! in the past. `headers == blocks` does not help either: both are frozen at
//! the same stale value.
//!
//! Every check we had passed in that state, and mining in it is not merely
//! unlucky — it is *certain* to be wasted, because the block would build on a
//! parent the network moved past long ago.
//!
//! # What actually catches it
//!
//! **Peer count.** A node with no peers cannot learn of a new block by any
//! means, so zero is unambiguous and cannot false-positive. This is the strong
//! signal.
//!
//! It carries extra weight on this chain. The fork kept Bitcoin's P2P magic and
//! default ports, so a node can connect to peers that are on the *other* side
//! of the hardfork. Peers are necessary, not sufficient — which is why the tip
//! age below is a real check here rather than a formality.
//!
//! **Tip age**, as a backstop for the case where peers exist but are not
//! delivering. This one needs a generous threshold, for two reasons: block
//! intervals are Poisson, so long gaps happen legitimately, and block
//! timestamps are not monotonic and may sit up to two hours in the future. At
//! three hours the chance of a false alarm is `e^-18`, about one in 67 million,
//! which is comfortably rarer than a wrong system clock.

use node_rpc::{BlockchainInfo, Network};

/// How far behind the present the tip may be before we refuse to mine.
///
/// Deliberately generous — see the module docs.
const MAX_TIP_AGE: i64 = 3 * 60 * 60;

/// How many blocks behind its own headers a node may be and still count as
/// caught up.
///
/// Not a fudge factor. `headers > blocks` is the **normal** state of a healthy
/// node: a header arrives, and for as long as it takes to download and validate
/// the block, the two counters differ. Treating any gap as "syncing" therefore
/// flags a node as unfit several times an hour, once per block.
///
/// That was a real outage here. This project's node shares block-download
/// bandwidth with the background chainstate validating history behind an
/// assumeutxo snapshot, which stretches the gap from milliseconds to seconds —
/// long enough for a periodic check to land inside it. The pool treated that as
/// fatal and exited, so mining stopped every few hours for a condition that
/// resolves on its own.
const AT_TIP_TOLERANCE: u32 = 2;

/// Why a node is not fit to mine on.
#[derive(Debug)]
pub enum NotReady {
    /// The node is running a different network than we asked for.
    WrongNetwork {
        /// What we asked for.
        wanted: String,
        /// What it is running.
        found: String,
    },
    /// The node says it is still in initial block download.
    ///
    /// Separate from [`Self::BehindHeaders`] because the two read very
    /// differently in a log. A node reporting IBD with `blocks == headers` is
    /// not "syncing 1578 of 1578" — it has everything it knows about and is
    /// waiting to decide it is caught up, usually because its tip is older than
    /// the 24-hour threshold Core uses.
    InitialBlockDownload {
        /// Blocks validated.
        blocks: u32,
    },
    /// The node has headers it has not yet validated blocks for.
    BehindHeaders {
        /// Blocks validated.
        blocks: u32,
        /// Headers known.
        headers: u32,
    },
    /// The node has no peers, so its view of the chain cannot advance.
    NoPeers,
    /// The tip is old enough that the node has plainly stopped keeping up.
    StaleTip {
        /// How many seconds behind the present.
        age: i64,
    },
}

impl std::fmt::Display for NotReady {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongNetwork { wanted, found } => {
                write!(f, "asked for {wanted} but the node is running {found}")
            }
            Self::InitialBlockDownload { blocks } => write!(
                f,
                "the node reports initial block download at height {blocks}. It has every \
                 block it knows of, so this usually means its tip is older than the \
                 24-hour threshold Core uses to call itself caught up — mine a block, or \
                 wait for a peer to send one"
            ),
            Self::BehindHeaders { blocks, headers } => write!(
                f,
                "the node is {} blocks behind its own headers ({blocks} of {headers}) — \
                 mining now would build on a stale tip",
                headers.saturating_sub(*blocks),
            ),
            Self::NoPeers => write!(
                f,
                "the node has no peers, so it cannot hear about new blocks. \
                 Its tip may look current while being hours old — Core latches \
                 `initialblockdownload` to false and never re-checks"
            ),
            Self::StaleTip { age } => write!(
                f,
                "the node's tip is {} hours old, so it has stopped keeping up. \
                 (If the chain really is quiet, check this machine's clock.)",
                age / 3600
            ),
        }
    }
}

impl std::error::Error for NotReady {}

/// Checks whether `info` describes a node worth mining on.
///
/// `peers` comes from `getconnectioncount`; `now` is the current Unix time.
pub fn check(
    network: Network,
    info: &BlockchainInfo,
    peers: u32,
    now: i64,
) -> Result<(), NotReady> {
    if info.chain != network.as_str() {
        return Err(NotReady::WrongNetwork {
            wanted: network.to_string(),
            found: info.chain.clone(),
        });
    }

    if info.initial_block_download {
        return Err(NotReady::InitialBlockDownload { blocks: info.blocks });
    }

    if info.headers.saturating_sub(info.blocks) > AT_TIP_TOLERANCE {
        return Err(NotReady::BehindHeaders {
            blocks: info.blocks,
            headers: info.headers,
        });
    }

    // Regtest is a private chain with no peers by design, and its blocks are
    // whenever you last mined one. Neither check means anything there.
    if network == Network::Regtest {
        return Ok(());
    }

    if peers == 0 {
        return Err(NotReady::NoPeers);
    }

    let age = now - info.time as i64;
    if age > MAX_TIP_AGE {
        return Err(NotReady::StaleTip { age });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(chain: &str, blocks: u32, headers: u32, ibd: bool, time: u64) -> BlockchainInfo {
        BlockchainInfo {
            chain: chain.to_owned(),
            blocks,
            headers,
            best_block_hash: String::new(),
            initial_block_download: ibd,
            pruned: true,
            time,
        }
    }

    const NOW: i64 = 2_000_000_000;

    #[test]
    fn a_healthy_node_passes() {
        let node = info("main", 900_000, 900_000, false, NOW as u64 - 300);
        assert!(check(Network::Mainnet, &node, 10, NOW).is_ok());
    }

    /// The case that motivated this module: everything the old checks looked at
    /// says "ready", because Core latches IBD to false and both counters froze
    /// together. Only the peer count reveals it.
    #[test]
    fn synced_looking_node_with_no_peers_is_rejected() {
        let node = info("main", 900_000, 900_000, false, NOW as u64 - 300);

        assert!(check(Network::Mainnet, &node, 10, NOW).is_ok(), "same node, with peers");
        assert!(
            matches!(check(Network::Mainnet, &node, 0, NOW), Err(NotReady::NoPeers)),
            "identical in every other respect, and unfit to mine on"
        );
    }

    #[test]
    fn an_old_tip_is_rejected_even_with_peers() {
        let node = info("main", 900_000, 900_000, false, NOW as u64 - 4 * 3600);
        assert!(matches!(
            check(Network::Mainnet, &node, 8, NOW),
            Err(NotReady::StaleTip { .. })
        ));
    }

    /// Block timestamps may legitimately sit up to two hours in the future, so
    /// a negative age must not be mistaken for a problem.
    #[test]
    fn a_future_tip_timestamp_is_fine() {
        let node = info("main", 900_000, 900_000, false, NOW as u64 + 7000);
        assert!(check(Network::Mainnet, &node, 8, NOW).is_ok());
    }

    /// A quiet-but-plausible gap must not trip the check.
    #[test]
    fn a_long_but_normal_gap_is_tolerated() {
        let node = info("main", 900_000, 900_000, false, NOW as u64 - 90 * 60);
        assert!(check(Network::Mainnet, &node, 8, NOW).is_ok());
    }

    /// Regtest has no peers and ancient blocks by design.
    #[test]
    fn regtest_is_exempt_from_peer_and_age_checks() {
        let node = info("regtest", 200, 200, false, 1_296_688_602);
        assert!(check(Network::Regtest, &node, 0, NOW).is_ok());
    }

    #[test]
    fn still_syncing_is_caught_by_either_signal() {
        let by_flag = info("main", 900_000, 900_000, true, NOW as u64);
        assert!(matches!(
            check(Network::Mainnet, &by_flag, 8, NOW),
            Err(NotReady::InitialBlockDownload { .. })
        ));

        // Headers well ahead of blocks, with the flag already latched false.
        let by_count = info("main", 899_000, 900_000, false, NOW as u64);
        assert!(matches!(
            check(Network::Mainnet, &by_count, 8, NOW),
            Err(NotReady::BehindHeaders { .. })
        ));
    }

    /// The two must not be confused, because their messages send a reader
    /// somewhere different. "1578 of 1578 blocks" read as a sync gap is
    /// nonsense; as an IBD flag on an old tip it is exactly the problem.
    #[test]
    fn ibd_with_no_gap_is_reported_as_ibd() {
        let node = info("main", 1578, 1578, true, NOW as u64);

        let Err(reason) = check(Network::Mainnet, &node, 8, NOW) else {
            panic!("a node in IBD is not fit to mine on");
        };
        assert!(matches!(reason, NotReady::InitialBlockDownload { .. }));
        assert!(
            !reason.to_string().contains("1578 of 1578"),
            "must not render an absent gap as though it were one: {reason}"
        );
    }

    /// The regression this tolerance exists for.
    ///
    /// A node one block behind its headers is mid-validation, not syncing, and
    /// that is the state it is in for a moment after every block the network
    /// finds. Flagging it stopped the pool several times a day.
    #[test]
    fn a_node_a_block_or_two_behind_is_still_at_the_tip() {
        for gap in 0..=AT_TIP_TOLERANCE {
            let node = info("main", 900_000 - gap, 900_000, false, NOW as u64 - 300);
            assert!(
                check(Network::Mainnet, &node, 8, NOW).is_ok(),
                "a {gap}-block gap should not count as syncing"
            );
        }

        // One past the tolerance does still count, so the check keeps its teeth.
        let node = info("main", 900_000 - (AT_TIP_TOLERANCE + 1), 900_000, false, NOW as u64 - 300);
        assert!(matches!(
            check(Network::Mainnet, &node, 8, NOW),
            Err(NotReady::BehindHeaders { .. })
        ));
    }

    #[test]
    fn the_wrong_network_is_caught_first() {
        let node = info("test", 900_000, 900_000, false, NOW as u64);
        assert!(matches!(
            check(Network::Mainnet, &node, 8, NOW),
            Err(NotReady::WrongNetwork { .. })
        ));
    }
}
