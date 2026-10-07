//! The screens that cover the dashboard: help, and a found block.

use events::clock;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::format;
use super::frame::Frame;
use super::state::State;
use crate::chain;
use crate::commands::{grouped, short_address};
use crate::config::network_key;

/// `?` — what everything on the dashboard means, in plain words.
pub fn help(state: &State, width: usize, height: usize) -> Vec<Line<'static>> {
    let speed = if state.rate > 0.0 {
        format!("It does about {} a second.", format::words(state.rate))
    } else {
        "It does millions a second.".to_owned()
    };
    let gap = if state.needed_zero_bits > state.best_zero_bits {
        format!("why {} of {} is nowhere near halfway.", state.best_zero_bits, state.needed_zero_bits)
    } else {
        "why being a few bits short is still very far away.".to_owned()
    };
    let topics = [
        ("HASH", format!(
            "Your computer scrambles the block's contents into a long number. \
             Each try is a \"hash\". {speed}"
        )),
        ("ZERO BITS", format!(
            "A block counts only if that number starts with enough zeros. Each \
             extra zero bit makes it twice as hard, which is {gap}"
        )),
        ("THE ODDS", "Every hash is a fresh lottery ticket. Past tries don't bring you \
             closer, so stopping and restarting costs nothing."
            .to_owned()),
        ("THE NODE", "Your own copy of the network's rules and history. It's what lets \
             you mine without trusting anyone."
            .to_owned()),
        ("REWARDS", "Paid only if you find a block — there is no partial credit, and \
             no slow drip of earnings. It's all or nothing."
            .to_owned()),
    ];

    let mut frame = Frame::new(width);
    frame.top(vec![Span::raw(format!("{} · what am I looking at?", chain::PROGRAM)).bold()], Vec::new());
    frame.blank();
    let text_width = frame.inner().saturating_sub(15);
    for (name, paragraph) in topics {
        for (index, line) in wrap(&paragraph, text_width).into_iter().enumerate() {
            let name = if index == 0 { name } else { "" };
            frame.row(vec![Span::raw(format!("  {name:<11}")).bold(), Span::raw(line)]);
        }
        frame.blank();
    }
    frame.fill_to(height, 1);
    frame.bottom("esc close")
}

/// Breaks `text` into lines of at most `columns`, between words.
fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let line = lines.last_mut().expect("never empty");
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > columns {
            lines.push(word.to_owned());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
}

/// The block-found screen.
pub fn found(state: &State, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut frame = Frame::new(width);
    let network = network_key(state.network).to_uppercase();
    frame.top(vec![Span::raw(chain::PROGRAM).bold()], vec![Span::raw(network)]);
    let Some(block) = &state.found else {
        frame.fill_to(height, 1);
        return frame.bottom("enter back to mining");
    };

    let centre = |text: String| {
        let indent = (width.saturating_sub(2 + text.chars().count())) / 2;
        format!("{}{text}", " ".repeat(indent))
    };
    frame.blank();
    frame.blank();
    frame.row(vec![Span::raw(centre("★  YOU FOUND A BLOCK  ★".into())).bold().yellow()]);
    frame.blank();
    frame.row(vec![Span::raw(centre(format!(
        "block {} · {} UTC · accepted by your node",
        grouped(block.height.into()),
        clock(block.time)
    )))]);
    frame.blank();

    let address = state.address.as_deref().map_or_else(|| "your address".to_owned(), short_address);
    let reward = block.reward.map_or_else(
        || "the block reward".to_owned(),
        |sats| format!("{} {}", format::coins(sats), chain::UNIT),
    );
    let maturity = chain::maturity(state.network, block.height);
    frame.row(vec![Span::raw(format!("          reward      {reward}  →  {address}"))]);
    frame.row(vec![Span::raw(format!(
        "          spendable   after block {} ({} from now)",
        grouped(u64::from(block.height + maturity)),
        chain::maturity_words(state.network, block.height)
    ))]);
    frame.blank();
    if let Some(odds) = block.odds.filter(|odds| *odds >= 1.0) {
        frame.row(vec![Span::raw(format!(
            "          The chance of that this session was about 1 in {}.",
            format::words(odds)
        ))]);
    }
    frame.blank();
    frame.rule(Vec::new());
    frame.row(vec![Span::raw("   Make sure the wallet that owns this address is backed up.")]);
    frame.fill_to(height, 1);
    frame.bottom("enter back to mining")
}
