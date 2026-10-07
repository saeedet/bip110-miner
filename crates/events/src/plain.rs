//! The events, printed as the lines the programs have always printed.
//!
//! Every format here is copied from the `println!` it replaced, and each is
//! pinned by a test against the exact original string. Anyone reading a log,
//! grepping one, or running a miner under a script should not be able to tell
//! that the output now goes through an event first.

use std::io::Write;

use crate::{Event, Level, Report, Sink, clock, si};

/// Which stream a line belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Ordinary output.
    Out,
    /// Warnings and failures.
    Err,
}

/// Prints events as plain lines on stdout and stderr.
pub struct Plain;

impl Sink for Plain {
    fn emit(&self, event: Event) {
        // One lock per event, so the lines of a multi-line event — a found
        // block, a status report — cannot be interleaved with another
        // thread's output. The old separate println! calls could be.
        let lines = render(&event);
        let stdout = std::io::stdout();
        let stderr = std::io::stderr();
        let mut out = stdout.lock();
        let mut err = stderr.lock();
        for (stream, text) in lines {
            let _ = match stream {
                Stream::Out => writeln!(out, "{text}"),
                Stream::Err => writeln!(err, "{text}"),
            };
        }
    }
}

/// The lines an event prints as, in order.
///
/// A line may itself contain a newline: several of the original messages
/// ended in `\n` to leave a blank line after them, and that is reproduced
/// rather than tidied up.
pub fn render(event: &Event) -> Vec<(Stream, String)> {
    use Stream::{Err, Out};

    let out = |text: String| vec![(Out, text)];

    match event {
        Event::Log { level, text } => {
            let stream = if *level == Level::Info { Out } else { Err };
            vec![(stream, text.clone())]
        }

        // --- pool -----------------------------------------------------------
        Event::PoolStarted { address, payout_script, network, listen, height, peers } => vec![
            (Out, format!("address : {address}")),
            (Out, format!("pool    : {network} on {listen}")),
            (Out, format!("node    : height {height} ({peers} peers)")),
            (Out, format!("payout  : {payout_script}\n")),
        ],
        Event::Listening { listen } => out(format!("waiting for miners on {listen}")),
        Event::WaitingForNode { reason } => out(format!("waiting for the node: {reason}")),
        Event::NewJob { height, job_id, transactions, miners, .. } => out(format!(
            "new tip at height {} — job {job_id} ({transactions} txs, {miners} miners)",
            height.saturating_sub(1),
        )),
        Event::MinimumDifficulty { difficulty, seconds_ahead } => out(if *seconds_ahead > 0 {
            format!(
                "  minimum difficulty ({difficulty:.0}) — stamping {}m{:02}s ahead of the wall clock",
                seconds_ahead / 60,
                seconds_ahead % 60,
            )
        } else {
            format!(
                "  minimum difficulty ({difficulty:.0}) — the window is open on its own, no clock pushed"
            )
        }),
        Event::DifficultyChanged { height, job_id, difficulty, miners } => out(format!(
            "DIFFICULTY CHANGED at height {height} — job {job_id} (difficulty {difficulty:.4}, {miners} miners)"
        )),
        Event::PoolReady => out("POOL READY — first job built, serving miners".to_owned()),
        Event::Paused { reason } => vec![
            (Out, format!("\npausing: {reason}")),
            (Out, "  waiting for the node — will resume on its own".to_owned()),
        ],
        Event::Resumed => out("node is fit to mine on again — resuming\n".to_owned()),
        Event::MinerConnected { peer, extranonce1 } => {
            out(format!("[{peer}] connected (extranonce1 {extranonce1})"))
        }
        Event::MinerAuthorized { peer, worker } => out(format!("[{peer}] authorized worker {worker}")),
        Event::MinerDisconnected { peer } => out(format!("[{peer}] disconnected")),
        Event::BlockFound { height, hash, peer } => vec![
            (Out, format!("\n*** BLOCK FOUND at height {height} ***")),
            (Out, format!("    {hash}")),
            (Out, format!("    submitted by {peer} and accepted by the node\n")),
        ],
        Event::BlockStale { peer, hash, reason } => {
            out(format!("[{peer}] block {hash} not adopted ({reason})"))
        }
        Event::BlockRejected { peer, hash, reason } => vec![
            (Err, format!("[{peer}] the node REJECTED a solved block: {reason}")),
            (Err, format!("    hash was {hash}")),
        ],
        Event::ShareChecked { peer, zero_bits, hash } => {
            out(format!("[{peer}] share {zero_bits} zero bits  {hash}"))
        }
        Event::BlockHeld { seconds } => out(format!(
            "  block solved early — holding {seconds}s until it can be submitted"
        )),

        // --- miner ----------------------------------------------------------
        Event::Connecting { pool } => out(format!("connecting to {pool}")),
        Event::Subscribed { extranonce1, extranonce2_size } => out(format!(
            "subscribed: extranonce1 {extranonce1}, extranonce2 {extranonce2_size} bytes"
        )),
        Event::Authorized { worker } => out(format!("authorized as {worker}\n")),
        Event::Hashing { threads, cores } => out(format!(
            "hashing on {threads} of {cores} cores{}\n",
            if threads >= cores { " (full tilt — expect heat)" } else { "" },
        )),
        Event::LifetimeLoaded { total_hashes, sessions, best_zero_bits } => out(format!(
            "lifetime so far: {} hashes over {sessions} sessions, best {best_zero_bits} zero bits\n",
            si(*total_hashes),
        )),
        Event::PoolDifficulty { difficulty } => out(format!("pool set difficulty to {difficulty}")),
        Event::JobReceived { job_id, difficulty, clean, prev_hash } => {
            let clean = if *clean { " (clean)" } else { "" };
            out(match prev_hash {
                Some(prev_hash) => format!("job {job_id} on {prev_hash}{clean}"),
                None => format!("job {job_id} at difficulty {difficulty:.4}{clean}"),
            })
        }
        Event::SolutionFound { hash, zero_bits } => {
            out(format!("solution found: {hash} ({zero_bits} zero bits)"))
        }
        Event::ShareAccepted => out("share accepted".to_owned()),
        Event::ShareRejected { reason } => vec![(Err, format!("share rejected: {reason}"))],
        Event::PoolClosed => out("the pool closed the connection".to_owned()),
        Event::Report(report) => render_report(report),
    }
}

/// The ruled status block.
fn render_report(report: &Report) -> Vec<(Stream, String)> {
    let mut lines = vec![
        (Stream::Out, format!("{} {} UTC", "─".repeat(52), clock(report.unix_time))),
        (
            Stream::Out,
            format!(
                "  {:>7.2} MH/s (avg {:>6.2})   session {:>8}   best {}/{} bits",
                report.recent_rate / 1e6,
                report.average_rate / 1e6,
                si(report.session_hashes),
                report.best_zero_bits,
                report.needed_zero_bits,
            ),
        ),
        (Stream::Out, format!("  {}", report.best_hash)),
    ];

    if let Some(lifetime) = &report.lifetime {
        // The shortfall as a power of two, because the bits themselves read
        // deceptively: 29 of 62 looks like halfway and is 2^33 short.
        let short = report.needed_zero_bits.saturating_sub(lifetime.best_zero_bits);
        lines.push((
            Stream::Out,
            format!(
                "  lifetime {:>8}   best ever {}/{} bits (2^{short} short)   ~1 in {:.3e} of a block",
                si(lifetime.total_hashes),
                lifetime.best_zero_bits,
                report.needed_zero_bits,
                lifetime.odds_denominator,
            ),
        ));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LifetimeReport;

    fn text(event: Event) -> String {
        render(&event).into_iter().map(|(_, line)| line).collect::<Vec<_>>().join("\n")
    }

    /// Lines copied from a real regtest session before events existed. If one
    /// of these changes, a log someone relies on has changed with it.
    #[test]
    fn pool_lines_match_the_originals() {
        assert_eq!(
            text(Event::NewJob { height: 18, job_id: "1".into(), transactions: 0, miners: 0, reward: 0 }),
            "new tip at height 17 — job 1 (0 txs, 0 miners)"
        );
        assert_eq!(text(Event::PoolReady), "POOL READY — first job built, serving miners");
        assert_eq!(
            text(Event::BlockFound { height: 2253, hash: "40ed".into(), peer: "127.0.0.1:52040".into() }),
            "\n*** BLOCK FOUND at height 2253 ***\n    40ed\n    submitted by 127.0.0.1:52040 and accepted by the node\n"
        );
        assert_eq!(
            text(Event::BlockStale { peer: "127.0.0.1:52040".into(), hash: "47e8".into(), reason: "inconclusive".into() }),
            "[127.0.0.1:52040] block 47e8 not adopted (inconclusive)"
        );
        assert_eq!(
            text(Event::MinimumDifficulty { difficulty: 1.0, seconds_ahead: 741 }),
            "  minimum difficulty (1) — stamping 12m21s ahead of the wall clock"
        );
    }

    #[test]
    fn miner_lines_match_the_originals() {
        assert_eq!(
            text(Event::Subscribed { extranonce1: "0000000000000001".into(), extranonce2_size: 8 }),
            "subscribed: extranonce1 0000000000000001, extranonce2 8 bytes"
        );
        assert_eq!(text(Event::Hashing { threads: 2, cores: 8 }), "hashing on 2 of 8 cores\n");
        assert_eq!(
            text(Event::Hashing { threads: 8, cores: 8 }),
            "hashing on 8 of 8 cores (full tilt — expect heat)\n"
        );
        assert_eq!(
            text(Event::JobReceived {
                job_id: "3".into(),
                difficulty: 1_141_943_013.515_522_2,
                clean: true,
                prev_hash: None,
            }),
            "job 3 at difficulty 1141943013.5155 (clean)"
        );
        // Bitcoin's form, where Stratum reveals the parent.
        assert_eq!(
            text(Event::JobReceived {
                job_id: "4c9".into(),
                difficulty: 1.0,
                clean: true,
                prev_hash: Some("28653d21".into()),
            }),
            "job 4c9 on 28653d21 (clean)"
        );
        assert_eq!(
            text(Event::PoolDifficulty { difficulty: 1_141_943_013.515_522_2 }),
            "pool set difficulty to 1141943013.5155222"
        );
    }

    /// The status block, against one printed by the miner on mainnet.
    #[test]
    fn the_report_matches_the_original_block() {
        let report = Report {
            unix_time: 1_789_392_434, // 13:27:14 UTC
            recent_rate: 9.44e6,
            average_rate: 10.24e6,
            session_hashes: 1_230_000_000,
            best_hash: "0000000eb5b0f142".into(),
            best_zero_bits: 28,
            needed_zero_bits: 62,
            lifetime: Some(LifetimeReport {
                total_hashes: 1_230_000_000,
                best_zero_bits: 28,
                odds_denominator: 3.991e9,
            }),
        };
        let lines: Vec<String> = render(&Event::Report(report)).into_iter().map(|(_, l)| l).collect();
        assert_eq!(lines[0], format!("{} 13:27:14 UTC", "─".repeat(52)));
        assert_eq!(lines[1], "     9.44 MH/s (avg  10.24)   session    1.23G   best 28/62 bits");
        assert_eq!(lines[2], "  0000000eb5b0f142");
        assert_eq!(
            lines[3],
            "  lifetime    1.23G   best ever 28/62 bits (2^34 short)   ~1 in 3.991e9 of a block"
        );
    }

    /// Failures go to stderr, exactly as eprintln! sent them.
    #[test]
    fn failures_go_to_stderr() {
        let rejected = render(&Event::BlockRejected { peer: "p".into(), hash: "h".into(), reason: "r".into() });
        assert!(rejected.iter().all(|(stream, _)| *stream == Stream::Err));
        assert_eq!(render(&Event::ShareRejected { reason: "x".into() })[0].0, Stream::Err);
        assert_eq!(render(&Event::Log { level: Level::Warn, text: "w".into() })[0].0, Stream::Err);
        assert_eq!(render(&Event::Log { level: Level::Info, text: "i".into() })[0].0, Stream::Out);
    }
}
