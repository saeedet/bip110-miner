//! The live dashboard: one screen, updated in place, with a few keys.
//!
//! [`run`] takes over the terminal until mining ends — from the keyboard, a
//! signal, or the session failing — and gives it back exactly as it found it,
//! even on a panic. Everything shown comes from [`State`], which is built from
//! the same events the plain output prints.

pub mod format;
pub mod frame;
mod screens;
mod state;
mod view;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use events::{Event, Level};
use miner::Controls;
use ratatui::crossterm::event::{self, Event as Input, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::Paragraph;
use serde_json::{Value, json};

use state::Overlay;
pub use state::{NodeStatus, State};

use crate::chain;
use crate::config::{self, Power, Settings};
use crate::node;

/// How often the node is asked how it is doing.
const NODE_POLL: Duration = Duration::from_secs(5);

/// How long to wait for a key before redrawing anyway.
const FRAME: Duration = Duration::from_millis(250);

/// What the dashboard needs from the session it shows.
pub struct Session<'a> {
    /// The settings mining runs with.
    pub settings: &'a Settings,
    /// Events from the pool and miner.
    pub events: Receiver<Event>,
    /// The miner's live controls.
    pub controls: Arc<Controls>,
    /// Set when mining has ended for any reason.
    pub finished: &'a dyn Fn() -> bool,
    /// Whether the machine is being kept awake.
    pub keeping_awake: bool,
    /// Anything worth saying before the first event arrives.
    pub notes: Vec<String>,
}

/// How the dashboard ended.
pub struct Outcome {
    /// What was on screen at the end, for the summary printed afterwards.
    pub state: State,
}

/// Shows the dashboard until mining ends or the user quits.
pub fn run(session: Session<'_>) -> Result<Outcome, String> {
    let settings = session.settings;
    let mut state = State::new(
        settings.network,
        (!settings.threads_from_flag).then_some(settings.power),
        settings.threads,
        cores(),
        unix_now(),
    );
    state.note(Level::Info, format!("{} {} started", chain::PROGRAM, env!("CARGO_PKG_VERSION")));
    if session.keeping_awake {
        state.note(Level::Info, "keeping this computer awake while mining (--allow-sleep to turn this off)");
    }
    for note in &session.notes {
        state.note(Level::Warn, note.clone());
    }

    let (node_updates, node_rx) = channel();
    let watching = Arc::new(AtomicBool::new(true));
    let watcher = watch_node(settings.clone(), node_updates, Arc::clone(&watching));

    // Restores the terminal on a panic too, so a crash never leaves it raw.
    let mut terminal = ratatui::init();
    let result = show(&mut terminal, &session, &mut state, &node_rx);
    ratatui::restore();

    watching.store(false, Ordering::Relaxed);
    let _ = watcher.join();
    result.map(|()| Outcome { state })
}

/// The draw-and-listen loop.
fn show(
    terminal: &mut ratatui::DefaultTerminal,
    session: &Session<'_>,
    state: &mut State,
    node: &Receiver<NodeStatus>,
) -> Result<(), String> {
    loop {
        state.now = unix_now();
        while let Ok(event) = session.events.try_recv() {
            state.apply(event);
        }
        while let Ok(status) = node.try_recv() {
            state.node = Some(status);
        }
        if (session.finished)() || session.controls.stopping() {
            return Ok(());
        }

        terminal
            .draw(|frame| {
                let area = frame.area();
                let lines = draw(state, usize::from(area.width), usize::from(area.height));
                frame.render_widget(Paragraph::new(lines), area);
            })
            .map_err(|error| format!("cannot draw: {error}"))?;

        if !event::poll(FRAME).map_err(|error| error.to_string())? {
            continue;
        }
        if let Input::Key(key) = event::read().map_err(|error| error.to_string())?
            && key.kind == KeyEventKind::Press
        {
            let quit = key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));
            if quit {
                return Ok(());
            }
            press(state, session, key.code);
        }
    }
}

/// The screen for a terminal of this size.
pub fn draw(state: &State, width: usize, height: usize) -> Vec<ratatui::text::Line<'static>> {
    if width < 60 || height < 16 {
        return vec![ratatui::text::Line::raw("Make this window bigger — the dashboard needs 80×24.")];
    }
    match state.overlay {
        Overlay::None => view::render(state, width, height),
        Overlay::Help => screens::help(state, width, height),
        Overlay::Found => screens::found(state, width, height),
    }
}

/// Acts on a key.
fn press(state: &mut State, session: &Session<'_>, key: KeyCode) {
    match (state.overlay, key) {
        (Overlay::None, KeyCode::Char('?')) => state.overlay = Overlay::Help,
        (Overlay::Help, KeyCode::Char('?') | KeyCode::Esc)
        | (Overlay::Found, KeyCode::Enter | KeyCode::Esc) => {
            state.overlay = Overlay::None;
        }
        (_, KeyCode::Char('p')) => {
            state.user_paused = !state.user_paused;
            session.controls.set_paused(state.user_paused);
            let said = if state.user_paused { "paused — press p to carry on" } else { "mining resumed" };
            state.note(Level::Info, said);
        }
        (_, KeyCode::Char('+' | '=')) => change_power(state, session, true),
        (_, KeyCode::Char('-' | '_')) => change_power(state, session, false),
        _ => {}
    }
}

/// Steps the power level up or down, applies it, and remembers it.
fn change_power(state: &mut State, session: &Session<'_>, up: bool) {
    const LEVELS: [Power; 3] = [Power::Eco, Power::Balanced, Power::Max];
    // An exact thread count from the command line steps from the middle.
    let index = LEVELS.iter().position(|level| Some(*level) == state.power).unwrap_or(1);
    let next = if up { LEVELS[(index + 1).min(2)] } else { LEVELS[index.saturating_sub(1)] };
    if state.power == Some(next) {
        return;
    }
    let threads = next.threads(state.cores);
    session.controls.set_threads(threads);
    state.power = Some(next);
    state.threads = threads;
    state.note(Level::Info, format!("power set to {} ({threads} of {} cores)", next.name(), state.cores));

    // Remembered for next time, as every other choice is.
    let path = config::path();
    let saved = config::File::load(&path).and_then(|mut file| {
        file.power = Some(next);
        file.save(&path)
    });
    if let Err(error) = saved {
        state.note(Level::Warn, format!("couldn't save the power setting: {error}"));
    }
}

/// Asks the node how it is doing every few seconds, on its own thread, so a
/// slow answer never freezes the screen.
fn watch_node(
    settings: Settings,
    updates: Sender<NodeStatus>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            if updates.send(node_status(&settings)).is_err() {
                return;
            }
            let mut waited = Duration::ZERO;
            while waited < NODE_POLL && running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                waited += Duration::from_millis(100);
            }
        }
    })
}

/// One look at the node.
fn node_status(settings: &Settings) -> NodeStatus {
    let Ok(client) = node::client(settings) else { return NodeStatus::default() };
    let Ok(info) = client.get_blockchain_info() else { return NodeStatus::default() };
    let peers: Vec<Value> = client.call("getpeerinfo", json!([])).unwrap_or_default();
    let raw: Value = client.call("getblockchaininfo", json!([])).unwrap_or(Value::Null);
    NodeStatus {
        running: true,
        blocks: info.blocks,
        headers: info.headers,
        initial_download: info.initial_block_download,
        tip_time: info.time,
        peers: peers.len(),
        chain_peers: chain::chain_peers(&peers),
        version: node::running_version(&client).map(node::version_string),
        disk_bytes: raw.get("size_on_disk").and_then(Value::as_f64),
    }
}

/// Logical cores on this machine.
fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod snapshots;
