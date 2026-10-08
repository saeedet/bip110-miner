//! Getting the blockchain: an optional quick start, then catching up.
//!
//! # The quick start
//!
//! A new node normally downloads and checks every block since 2009, which
//! takes days. With a UTXO snapshot it can instead load the set of unspent
//! coins as of a recent block and start from there, then re-check the older
//! history in the background while it mines.
//!
//! The snapshot comes from a mirror, but the mirror is not trusted: the node
//! has the snapshot's hash compiled in and refuses one that differs by a
//! single bit (`loadtxoutset`).
//!
//! # Progress
//!
//! Measured with the node's own estimate, which weighs blocks by the
//! transactions in them. A count of blocks would race through the nearly
//! empty early years and then crawl.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use node_rpc::RpcClient;
use ratatui::text::Line;
use serde_json::{Value, json};

use super::download::{self, Fetched, bar};
use super::screen::{Answer, Part, Wizard};
use crate::chain::{self, Snapshot};
use crate::commands::grouped;
use crate::config::Settings;
use crate::dashboard::format;
use crate::{node, platform};

/// Room needed beyond the snapshot itself: the chainstate it becomes, and
/// the blocks kept after it.
const ROOM_BESIDE_SNAPSHOT: u64 = 20_000_000_000;

/// How long `loadtxoutset` may take before the call is given up on. The node
/// carries on regardless.
const LOAD_TIMEOUT: Duration = Duration::from_secs(4 * 3_600);

/// Waits for the node to catch up, offering the quick start first.
pub fn wait(wizard: &mut Wizard, settings: &Settings, client: &RpcClient) -> Result<Answer<()>, String> {
    let info = blockchain_info(client)?;
    if caught_up(&info) {
        return Ok(Answer::Given(()));
    }

    if let Some(snapshot) = chain::snapshot(settings.network)
        && worth_offering(client, &info, &snapshot, settings)
    {
        match offer(wizard, settings, client, &snapshot)? {
            Answer::Quit => return Ok(Answer::Quit),
            Answer::Given(()) | Answer::Back => {}
        }
    }
    follow(wizard, settings, client)
}

/// `getblockchaininfo`, as raw JSON: the progress fields matter here.
fn blockchain_info(client: &RpcClient) -> Result<Value, String> {
    client.call("getblockchaininfo", json!([])).map_err(|error| error.to_string())
}

fn number(info: &Value, field: &str) -> u64 {
    info.get(field).and_then(Value::as_u64).unwrap_or(0)
}

/// Caught up enough to mine: the same test the pool uses.
fn caught_up(info: &Value) -> bool {
    let ibd = info.get("initialblockdownload").and_then(Value::as_bool).unwrap_or(true);
    let (blocks, headers) = (number(info, "blocks"), number(info, "headers"));
    !ibd && headers > 0 && headers <= blocks + 2
}

/// Whether the quick start would help: early enough, not done already, and
/// with the disk to hold it.
fn worth_offering(client: &RpcClient, info: &Value, snapshot: &Snapshot, settings: &Settings) -> bool {
    let early = number(info, "blocks") < u64::from(snapshot.height);
    let states: Value = client.call("getchainstates", json!([])).unwrap_or(Value::Null);
    let one_chainstate =
        states.get("chainstates").and_then(Value::as_array).is_none_or(|states| states.len() < 2);
    let room = platform::free_bytes(&settings.datadir)
        .is_none_or(|free| free > snapshot.bytes + ROOM_BESIDE_SNAPSHOT);
    early && one_chainstate && room
}

/// Asks about the quick start, and does it if wanted.
fn offer(
    wizard: &mut Wizard,
    settings: &Settings,
    client: &RpcClient,
    snapshot: &Snapshot,
) -> Result<Answer<()>, String> {
    let intro = [
        Part::Text(
            "Before mining, your computer needs its own copy of the blockchain, so it can check every \
             block itself instead of trusting anyone else."
                .into(),
        ),
        Part::Gap,
        Part::Text(format!(
            "Checking every block since 2009 takes days. A quick start loads a {} snapshot of who \
             owns what at block {}, checked by your node against a fingerprint built into it, and \
             gets you mining in hours. The older history is re-checked in the background afterwards.",
            download::size(snapshot.bytes),
            grouped(snapshot.height.into())
        )),
    ];
    let choices = vec![
        format!("Quick start: download the snapshot   ({})   (recommended)", download::size(snapshot.bytes)),
        "Check everything from the start — slower, nothing extra to download".to_owned(),
    ];
    match wizard.choose(&intro, &choices)? {
        Answer::Given(0) => quick_start(wizard, settings, client, snapshot),
        Answer::Given(_) | Answer::Back => Ok(Answer::Given(())),
        Answer::Quit => Ok(Answer::Quit),
    }
}

/// Downloads the snapshot, waits for the node to know its block, and loads it.
fn quick_start(
    wizard: &mut Wizard,
    settings: &Settings,
    client: &RpcClient,
    snapshot: &Snapshot,
) -> Result<Answer<()>, String> {
    let file: PathBuf = platform::state_dir().join("snapshots").join(snapshot.file);
    let intro =
        [Part::Text(format!("Downloading the snapshot of block {}…", grouped(snapshot.height.into())))];
    if let Fetched::Stopped = download::fetch(wizard, &intro, snapshot.url, &file, snapshot.bytes)? {
        return Ok(Answer::Quit);
    }

    // The node must know the snapshot's block before it can load it, which
    // takes a few minutes of downloading headers on a new node.
    let known = wizard.watch("q quit (nothing is lost)", || {
        let info = blockchain_info(client)?;
        if number(&info, "headers") >= u64::from(snapshot.height) {
            return Ok(None);
        }
        Ok(Some(vec![Part::Text(format!(
            "Waiting for the node to hear about block {} — it knows of {} so far.",
            grouped(snapshot.height.into()),
            grouped(number(&info, "headers"))
        ))]))
    })?;
    if !known {
        return Ok(Answer::Quit);
    }

    let loader = node::client(settings)?.with_timeout(LOAD_TIMEOUT);
    let path = file.display().to_string();
    let loading = std::thread::spawn(move || loader.call::<Value>("loadtxoutset", json!([path])));
    let started = Instant::now();
    let finished = wizard.watch("q quit (the node finishes loading it anyway)", || {
        if loading.is_finished() {
            return Ok(None);
        }
        Ok(Some(vec![
            Part::Text(
                "Your node is checking the snapshot and loading it. This usually takes 10 to 30 minutes."
                    .into(),
            ),
            Part::Gap,
            Part::Text(format!("{} so far.", format::duration(started.elapsed().as_secs_f64()))),
        ]))
    })?;
    if !finished {
        return Ok(Answer::Quit);
    }

    match loading.join().map_err(|_| "loading the snapshot stopped unexpectedly")? {
        Ok(_) => {
            // Loaded into the node's own database; the file has done its job.
            let _ = std::fs::remove_file(&file);
            Ok(Answer::Given(()))
        }
        Err(error) => {
            let body = [
                Part::Text(format!("The node did not accept the snapshot: {error}")),
                Part::Gap,
                Part::Text("Nothing is lost — it will check everything from the start instead.".into()),
            ];
            Ok(if wizard.notice(&body, "enter continue · q quit")? {
                Answer::Given(())
            } else {
                Answer::Quit
            })
        }
    }
}

/// The catching-up screen, until the node is in sync.
fn follow(wizard: &mut Wizard, settings: &Settings, client: &RpcClient) -> Result<Answer<()>, String> {
    let mut history: VecDeque<(Instant, f64)> = VecDeque::new();
    let mut last_poll: Option<(Instant, Value, Value)> = None;
    let caught = wizard.watch("q quit (progress is kept) · mining starts on its own", || {
        // The node is asked every two seconds, not on every redraw.
        let stale = last_poll.as_ref().is_none_or(|(at, _, _)| at.elapsed() >= Duration::from_secs(2));
        if stale {
            let info = blockchain_info(client)?;
            let states: Value = client.call("getchainstates", json!([])).unwrap_or(Value::Null);
            last_poll = Some((Instant::now(), info, states));
        }
        let (_, info, states) = last_poll.as_ref().expect("just polled");
        if caught_up(info) {
            return Ok(None);
        }
        let progress = info.get("verificationprogress").and_then(Value::as_f64).unwrap_or(0.0);
        if stale {
            history.push_back((Instant::now(), progress));
            while history.len() > 120 {
                history.pop_front();
            }
        }
        Ok(Some(progress_body(info, states, settings, &history)))
    })?;
    Ok(if caught { Answer::Given(()) } else { Answer::Quit })
}

/// The time left, from how fast progress has been moving lately.
fn time_left(history: &VecDeque<(Instant, f64)>) -> Option<f64> {
    let (first, last) = (history.front()?, history.back()?);
    let seconds = last.0.duration_since(first.0).as_secs_f64();
    let gained = last.1 - first.1;
    (seconds > 20.0 && gained > 0.0).then(|| (1.0 - last.1) / (gained / seconds))
}

/// Screen 5's body.
fn progress_body(
    info: &Value,
    states: &Value,
    settings: &Settings,
    history: &VecDeque<(Instant, f64)>,
) -> Vec<Part> {
    let progress = info.get("verificationprogress").and_then(Value::as_f64).unwrap_or(0.0);
    let (blocks, headers) = (number(info, "blocks"), number(info, "headers"));
    let left = time_left(history)
        .map_or_else(|| "working it out…".to_owned(), |s| format!("about {} left", format::duration(s)));
    let background = states
        .get("chainstates")
        .and_then(Value::as_array)
        .filter(|states| states.len() > 1)
        .and_then(|states| states.iter().find(|state| state.get("snapshot_blockhash").is_none()));

    let mut body = vec![
        Part::Text(
            "Before mining, your computer needs its own copy of the blockchain, so it can check every \
             block itself instead of trusting anyone else."
                .into(),
        ),
        Part::Gap,
        Part::Line(Line::raw(format!(
            "    {:<17}{}  {:>3}%",
            "Catching up",
            bar(progress, 28),
            (progress * 100.0).floor() as u64
        ))),
        Part::Line(Line::raw(format!(
            "    {:<17}block {} of {} · {left}",
            "",
            grouped(blocks),
            if headers > 0 { grouped(headers) } else { "…".to_owned() }
        ))),
        Part::Gap,
        Part::Text("⏵ Mining starts on its own when this reaches 100%.".into()),
    ];
    if let Some(background) = background {
        let checked = background.get("verificationprogress").and_then(Value::as_f64).unwrap_or(0.0);
        body.extend([
            Part::Gap,
            Part::Line(Line::raw(format!(
                "    {:<17}{}  {:>3}%   in the background",
                "Checking history",
                bar(checked, 28),
                (checked * 100.0).floor() as u64
            ))),
            Part::Line(Line::raw(format!(
                "    {:<17}re-checks every block since 2009 · you can mine meanwhile",
                ""
            ))),
        ]);
    }
    let used = info
        .get("size_on_disk")
        .and_then(Value::as_u64)
        .map_or_else(String::new, |bytes| format!("{} used", download::size(bytes)));
    let free = platform::free_bytes(&settings.datadir)
        .map_or_else(String::new, |bytes| format!(" · {} free", download::size(bytes)));
    body.extend([
        Part::Gap,
        Part::Text(format!("Disk {used}{free}")),
        Part::Text("Safe to quit at any time: progress is kept and picks up next run.".into()),
    ]);
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caught_up_means_what_the_pool_means() {
        assert!(caught_up(&json!({"initialblockdownload": false, "blocks": 100, "headers": 102})));
        assert!(!caught_up(&json!({"initialblockdownload": false, "blocks": 100, "headers": 103})));
        assert!(!caught_up(&json!({"initialblockdownload": true, "blocks": 100, "headers": 100})));
        assert!(!caught_up(&json!({"initialblockdownload": false, "blocks": 0, "headers": 0})));
    }

    #[test]
    fn no_estimate_until_there_is_something_to_go_on() {
        let start = Instant::now();
        let mut history = VecDeque::from([(start, 0.10)]);
        assert_eq!(time_left(&history), None);
        history.push_back((start + Duration::from_secs(60), 0.11));
        let left = time_left(&history).expect("a minute of progress is enough");
        assert!((left - 89.0 * 60.0).abs() < 1.0, "{left}");
    }
}
