//! The dashboard: one screen, redrawn in place.
//!
//! Two layouts, both from `docs/cli-design.md`: a single column for an 80×24
//! terminal, and two columns side by side from 110 columns up. Either way the
//! RECENT section grows to fill whatever height is left.

use events::{Level, clock};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use super::format;
use super::frame::Frame;
use super::state::{PoolState, State};
use crate::chain;
use crate::commands::{grouped, short_address};
use crate::config::{Power, network_key};

/// From this many columns, the two-column layout.
const WIDE: usize = 110;

/// The key hints along the bottom.
const KEYS: &str = "q quit · p pause · +/- power · ? what am I looking at";

/// Draws the dashboard for a terminal `width` × `height`.
pub fn render(state: &State, width: usize, height: usize) -> Vec<Line<'static>> {
    if width >= WIDE { wide(state, width, height) } else { narrow(state, width, height) }
}

fn narrow(state: &State, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut frame = Frame::new(width);
    frame.top(title(), header(state, false));
    frame.row(node_line(state, false));

    frame.rule(mining_label(state));
    frame.row(rate_line(state, 36, "last 10 min", 600));
    let mut power = vec![Span::raw("   power  ")];
    power.extend(power_summary(state));
    let mut job = vec![Span::raw("job: ")];
    job.extend(job_summary(state, true));
    let gap = 44usize.saturating_sub(super::frame::width(&power));
    power.push(Span::raw(" ".repeat(gap.max(2))));
    power.extend(job);
    frame.row(power);

    frame.rule(label("YOUR CHANCES"));
    for (name, value) in chances(state, false) {
        frame.row(vec![Span::raw(format!("   {name:<16}")), value]);
    }

    frame.rule(label("ALL TIME"));
    frame.row(vec![Span::raw(format!("   {}", all_time(state, false)))]);
    frame.row(vec![Span::raw(format!("   {}", rewards(state, false)))]);

    frame.rule(label("RECENT"));
    recent(&mut frame, state, height, false);
    frame.bottom(KEYS)
}

fn wide(state: &State, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut frame = Frame::new(width);
    let left = (frame.inner() - 1) / 2;
    frame.top(title(), header(state, true));
    frame.row(node_line(state, true));

    frame.split(mining_label(state), label("YOUR CHANCES"), left);
    let mut mining = vec![
        Vec::new(),
        rate_headline(state),
        vec![Span::raw("   "), spark(state, 40, 1_800)],
        vec![Span::raw("   last 30 minutes").dark_gray()],
        Vec::new(),
    ];
    let mut power = vec![Span::raw("   power   ")];
    power.extend(power_choices(state));
    mining.push(power);
    mining.push(vec![Span::raw(format!(
        "           {} of {} cores · press + or - to change",
        state.threads, state.cores
    ))]);
    mining.push(Vec::new());
    let mut job = vec![Span::raw("   job     ")];
    job.extend(job_summary(state, false));
    mining.push(job);
    if let Some(built) = &state.job {
        mining.push(vec![Span::raw(format!(
            "           built by your node {} ago",
            format::duration(state.now.saturating_sub(built.since) as f64)
        ))]);
    }

    let mut chances_column = vec![Vec::new()];
    for (name, value) in chances(state, true) {
        if !name.is_empty() {
            chances_column.push(vec![Span::raw(format!("   {name}"))]);
        }
        chances_column.push(vec![Span::raw("      "), value]);
        if name.is_empty() || name == "On average" || name == "This session" {
            chances_column.push(Vec::new());
        }
    }
    let rows = mining.len().max(chances_column.len()).max(11);
    for row in 0..rows {
        frame.columns(
            mining.get(row).cloned().unwrap_or_default(),
            chances_column.get(row).cloned().unwrap_or_default(),
            left,
        );
    }

    frame.join(label("ALL TIME"), left);
    frame.row(vec![Span::raw(format!("   {}", all_time(state, true)))]);
    frame.row(vec![Span::raw(format!("   {}", rewards(state, true)))]);

    frame.rule(label("RECENT"));
    recent(&mut frame, state, height, true);
    frame.bottom(KEYS)
}

fn title() -> Vec<Span<'static>> {
    vec![Span::raw(chain::PROGRAM).bold()]
}

fn label(text: &str) -> Vec<Span<'static>> {
    vec![Span::raw(text.to_owned()).bold()]
}

/// `MAINNET · 14:23 UTC · up 2h14m`
fn header(state: &State, wide: bool) -> Vec<Span<'static>> {
    let time = clock(state.now);
    let time = if wide { time } else { time[..5].to_owned() };
    let network = network_key(state.network).to_uppercase();
    vec![Span::raw(format!(
        "{network} · {time} UTC · up {}",
        format::uptime(state.now.saturating_sub(state.started), wide)
    ))]
}

/// `NODE  ● in sync · block 976,034 · 12 peers · Knots 29.4.2 · 5.1 GB`
fn node_line(state: &State, wide: bool) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("  NODE  ").bold()];
    let Some(node) = &state.node else {
        spans.push(Span::raw("○ checking…").dark_gray());
        return spans;
    };
    if !node.running {
        spans.push(Span::raw("● not answering").red());
        return spans;
    }
    let mut facts = Vec::new();
    if node.in_sync() {
        spans.push(Span::raw("● in sync").green());
        facts.push(format!("block {}", grouped(node.blocks.into())));
        if wide {
            let age = state.now.saturating_sub(node.tip_time) / 60;
            facts.push(format!("tip {age} min old"));
        }
    } else {
        spans.push(Span::raw("● catching up").yellow());
        facts.push(format!("block {} of {}", grouped(node.blocks.into()), grouped(node.headers.into())));
    }
    facts.push(match (wide, node.chain_peers) {
        (true, Some(ours)) => format!("{} peers ({ours} {})", node.peers, chain::CHAIN_LABEL),
        _ => format!("{} peers", node.peers),
    });
    // Catching up, the block count says more and needs the room.
    if wide || node.in_sync() {
        if let Some(version) = &node.version {
            facts.push(format!("{} {version}", chain::NODE_NAME));
        }
        if let Some(bytes) = node.disk_bytes {
            facts.push(format!("{:.1} GB", bytes / 1e9));
        }
    }
    spans.push(Span::raw(format!(" · {}", facts.join(" · "))));
    spans
}

/// `MINING`, with why it isn't when it isn't.
fn mining_label(state: &State) -> Vec<Span<'static>> {
    let mut spans = label("MINING");
    if state.user_paused {
        spans.push(Span::raw(" ■ paused").yellow());
    } else if matches!(state.pool, PoolState::Waiting(_) | PoolState::Paused(_)) {
        spans.push(Span::raw(" ■ waiting for the node").yellow());
    }
    spans
}

/// Why nothing is being hashed, if nothing is.
fn idle_reason(state: &State) -> Option<Span<'static>> {
    if state.user_paused {
        return Some(Span::raw("paused — press p to carry on").yellow());
    }
    match &state.pool {
        PoolState::Waiting(reason) | PoolState::Paused(reason) => Some(Span::raw(reason.clone()).yellow()),
        PoolState::Starting => Some(Span::raw("starting…").dark_gray()),
        PoolState::Ready if state.rate == 0.0 => Some(Span::raw("starting…").dark_gray()),
        PoolState::Ready => None,
    }
}

/// `26.1 MH/s  ▁▂▃▅▆▇▇▆  last 10 min`
fn rate_line(state: &State, cells: usize, caption: &str, window: u64) -> Vec<Span<'static>> {
    if let Some(reason) = idle_reason(state) {
        return vec![Span::raw("   "), reason];
    }
    vec![
        Span::raw("   "),
        Span::raw(format::rate(state.rate)).bold().cyan(),
        Span::raw("  "),
        spark(state, cells, window),
        Span::raw(format!("  {caption}")).dark_gray(),
    ]
}

/// `26.1 MH/s   (average 25.8 this session)`
fn rate_headline(state: &State) -> Vec<Span<'static>> {
    if let Some(reason) = idle_reason(state) {
        return vec![Span::raw("   "), reason];
    }
    vec![
        Span::raw("   "),
        Span::raw(format::rate(state.rate)).bold().cyan(),
        Span::raw(format!("   (average {} this session)", format::rate(state.average))).dark_gray(),
    ]
}

/// The hashrate over the last `window` seconds, as `cells` bars.
fn spark(state: &State, cells: usize, window: u64) -> Span<'static> {
    Span::styled(sparkline(&state.samples, state.now, window, cells), Style::new().cyan())
}

/// Bars for the average rate in each slice of the window; blank where there
/// is no data yet, so a young session doesn't pretend to a history it lacks.
pub fn sparkline(
    samples: &std::collections::VecDeque<(u64, f64)>,
    now: u64,
    window: u64,
    cells: usize,
) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let start = now.saturating_sub(window);
    let slice = (window as f64 / cells as f64).max(1.0);
    let mut buckets = vec![(0.0, 0u32); cells];
    for &(time, rate) in samples {
        if time < start || time > now {
            continue;
        }
        let index = (((time - start) as f64 / slice) as usize).min(cells - 1);
        buckets[index].0 += rate;
        buckets[index].1 += 1;
    }
    let averages: Vec<Option<f64>> =
        buckets.iter().map(|&(sum, count)| (count > 0).then(|| sum / f64::from(count))).collect();
    let peak = averages.iter().flatten().copied().fold(0.0, f64::max);
    averages
        .iter()
        .map(|average| match average {
            None => ' ',
            Some(_) if peak <= 0.0 => BARS[0],
            Some(value) => BARS[((value / peak) * 7.0).round() as usize],
        })
        .collect()
}

/// `● balanced (6 of 8 cores)`
fn power_summary(state: &State) -> Vec<Span<'static>> {
    let name = state.power.map_or("custom", Power::name);
    vec![Span::raw("● ").green(), Span::raw(format!("{name} ({} of {} cores)", state.threads, state.cores))]
}

/// `○ eco   ● balanced   ○ max`
fn power_choices(state: &State) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, power) in [Power::Eco, Power::Balanced, Power::Max].into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        if state.power == Some(power) {
            spans.push(Span::raw("● ").green());
            spans.push(Span::raw(power.name()).bold());
        } else {
            spans.push(Span::raw(format!("○ {}", power.name())).dark_gray());
        }
    }
    spans
}

/// `block 976,035 · 203 txs`, or `transactions` in full when `short` is off.
fn job_summary(state: &State, short: bool) -> Vec<Span<'static>> {
    match &state.job {
        Some(job) => vec![Span::raw(format!(
            "block {} · {} {}",
            grouped(job.height.into()),
            grouped(job.transactions as u64),
            if short { "txs" } else { "transactions" }
        ))],
        None => vec![Span::raw("no job yet").dark_gray()],
    }
}

/// The three YOUR CHANCES lines, as (name, value). The value of the last one
/// spills onto a second line, named "". `wide` has room for fuller wording.
fn chances(state: &State, wide: bool) -> Vec<(&'static str, Span<'static>)> {
    let session = match state.session_odds() {
        None => Span::raw("waiting for the first job").dark_gray(),
        Some(odds) if odds < 1.0 => {
            Span::raw(format!("about {} blocks' worth of work so far", format::words(1.0 / odds)))
        }
        Some(odds) => Span::raw(format!("about 1 in {} of finding a block", format::words(odds))),
    };
    let average = if state.rate > 0.0 && state.difficulty > 0.0 {
        Span::raw(format!(
            "one block every ≈ {} at this speed",
            format::duration(state.expected_hashes() / state.rate)
        ))
    } else {
        Span::raw("—").dark_gray()
    };
    let needed = state.needed_zero_bits;
    let best = if needed == 0 {
        Span::raw("—").dark_gray()
    } else if state.best_zero_bits >= needed {
        // Regtest needs a single zero bit; "11 of the 1" would read as nonsense.
        Span::raw(format!("{} zero bits — a block needs only {needed}", state.best_zero_bits))
    } else {
        let ending = if wide { "a block needs" } else { "needed" };
        Span::raw(format!("{} of the {needed} zero bits {ending}", state.best_zero_bits))
    };
    let short = state.needed_zero_bits.saturating_sub(state.best_zero_bits);
    let doubling = if short == 0 {
        Span::raw("")
    } else {
        let text = format!(
            "each missing bit doubles it: 2{} ≈ {}× short",
            format::superscript(short),
            format::words(2f64.powi(short as i32))
        );
        Span::raw(if wide { text } else { format!("({text})") }).dark_gray()
    };
    let best_name = if wide { "Best hash this session" } else { "Best hash yet" };
    vec![("This session", session), ("On average", average), (best_name, best), ("", doubling)]
}

/// `3.04 T hashes · 12 sessions · best ever 34 bits`
fn all_time(state: &State, wide: bool) -> String {
    let Some(lifetime) = &state.lifetime else { return "counting…".to_owned() };
    let mut text = format!(
        "{} hashes · {} session{} · best ever {} zero bits",
        events::si(lifetime.total_hashes),
        grouped(lifetime.sessions),
        if lifetime.sessions == 1 { "" } else { "s" },
        lifetime.best_zero_bits,
    );
    if wide && state.difficulty > 0.0 && lifetime.total_hashes > 0 {
        let odds = state.expected_hashes() / lifetime.total_hashes as f64;
        if odds >= 1.0 {
            text.push_str(&format!(" · about 1 in {} of a block so far", format::words(odds)));
        }
    }
    text
}

/// `rewards to bc1qw50…f3t4 · spendable 45 days after a block is found`
fn rewards(state: &State, wide: bool) -> String {
    let Some(address) = &state.address else { return "rewards: waiting for the pool".to_owned() };
    let address = if wide { address.clone() } else { short_address(address) };
    let height = state.job.as_ref().map_or(0, |job| job.height);
    format!(
        "rewards to {address} · spendable {} after a block is found",
        chain::maturity_words(state.network, height)
    )
}

/// RECENT, newest first, as many as fit.
fn recent(frame: &mut Frame, state: &State, height: usize, wide: bool) {
    let room = height.saturating_sub(frame.len() + 1);
    for entry in state.recent.iter().take(room) {
        let time = clock(entry.time);
        let time = if wide { time } else { time[..5].to_owned() };
        let text = Span::raw(entry.text.clone());
        let text = match entry.level {
            Level::Info => text,
            Level::Warn => text.yellow(),
            Level::Fatal => text.red(),
        };
        frame.row(vec![Span::raw(format!("   {time}  ")).dark_gray(), text]);
    }
    frame.fill_to(height, 1);
}
