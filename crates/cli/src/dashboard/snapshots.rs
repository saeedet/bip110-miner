//! Whole-screen snapshots.
//!
//! Each test draws a screen for a fixed state and compares it, as text, with
//! a file under `snapshots/`. A layout change shows up as a readable diff. To
//! accept a deliberate change, rerun with `UPDATE_SNAPSHOTS=1` and review the
//! files before committing them.

use std::collections::VecDeque;

use events::Level;
use node_rpc::Network;

use super::draw;
use super::frame::text;
use super::state::{Entry, Found, Job, Lifetime, NodeStatus, Overlay, PoolState, State};
use crate::config::Power;

/// 2026-09-14 14:23:05 UTC.
const NOW: u64 = 1_789_395_785;

/// A session two and a quarter hours in, roughly as in the design.
fn mining() -> State {
    let mut state = State::new(Network::Mainnet, Some(Power::Balanced), 6, 8, NOW - 8_040);
    state.now = NOW;
    state.node = Some(NodeStatus {
        running: true,
        blocks: 976_034,
        headers: 976_034,
        initial_download: false,
        tip_time: NOW - 180,
        peers: 12,
        chain_peers: Some(9),
        version: Some("29.4.2".into()),
        disk_bytes: Some(5.1e9),
    });
    state.pool = PoolState::Ready;
    state.rate = 26.1e6;
    state.average = 25.8e6;
    // Eco for a while, then balanced: the graph should show the step.
    state.samples = (0..900u64)
        .map(|i| {
            let base = if i < 400 { 8.7e6 } else { 25.4e6 };
            (NOW - 1_800 + i * 2, base + (i % 11) as f64 * 0.12e6)
        })
        .collect();
    state.session_hashes = 207_432_000_000;
    state.best_zero_bits = 28;
    state.needed_zero_bits = 62;
    state.difficulty = 1.14e9;
    state.job = Some(Job { height: 976_035, transactions: 203, reward: 312_620_000, since: NOW - 52 });
    state.lifetime = Some(Lifetime { total_hashes: 3_040_000_000_000, sessions: 12, best_zero_bits: 34 });
    state.address = Some("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into());
    state.recent = [
        (NOW - 14, "someone else found block 976,034 → new job, nothing lost"),
        (NOW - 476, "someone else found block 976,033 → new job, nothing lost"),
        (NOW - 658, "new personal best: 28 zero bits"),
        (NOW - 741, "power set to balanced (6 of 8 cores)"),
        (NOW - 805, "mining started"),
        (NOW - 874, "waiting for the node: 4 blocks behind its own headers"),
        (NOW - 903, "bip110-miner 0.1.0 started"),
    ]
    .into_iter()
    .map(|(time, text)| Entry { time, level: Level::Info, text: text.into() })
    .collect::<VecDeque<_>>();
    state
}

/// Compares a screen with its snapshot, or rewrites the snapshot.
fn check(name: &str, width: usize, height: usize, state: &State) {
    let lines = draw(state, width, height);
    let actual = text(&lines);

    assert_eq!(lines.len(), height, "{name}: every row of the terminal is drawn");
    for (row, line) in actual.lines().enumerate() {
        assert_eq!(line.chars().count(), width, "{name}: row {row} is not {width} wide: {line:?}");
    }

    let path = format!("{}/src/dashboard/snapshots/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, format!("{actual}\n")).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected.trim_end_matches('\n') == actual,
        "{name} changed. Rerun with UPDATE_SNAPSHOTS=1 to accept.\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

#[test]
fn dashboard_80x24() {
    check("dashboard_80x24", 80, 24, &mining());
}

#[test]
fn dashboard_120x40() {
    check("dashboard_120x40", 120, 40, &mining());
}

#[test]
fn dashboard_waiting_for_the_node() {
    let mut state = mining();
    if let Some(node) = state.node.as_mut() {
        node.blocks = 976_030;
    }
    state.pool = PoolState::Paused("the node is 4 blocks behind its own headers".into());
    check("dashboard_waiting_80x24", 80, 24, &state);
}

#[test]
fn dashboard_paused_by_the_user() {
    let mut state = mining();
    state.user_paused = true;
    check("dashboard_paused_80x24", 80, 24, &state);
}

#[test]
fn a_fresh_start_promises_nothing() {
    let mut state = State::new(Network::Mainnet, Some(Power::Eco), 2, 8, NOW);
    state.note(Level::Info, "bip110-miner 0.1.0 started");
    check("dashboard_starting_80x24", 80, 24, &state);
}

#[test]
fn found_a_block() {
    let mut state = mining();
    state.found = Some(Found {
        height: 976_035,
        hash: "00".repeat(32),
        time: NOW,
        reward: Some(312_620_000),
        odds: Some(2.36e7),
    });
    state.overlay = Overlay::Found;
    check("found_80x24", 80, 24, &state);
}

#[test]
fn help() {
    let mut state = mining();
    state.overlay = Overlay::Help;
    check("help_80x24", 80, 24, &state);
}
