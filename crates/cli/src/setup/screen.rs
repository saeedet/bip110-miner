//! The setup screens: a checklist on top, the current question below.
//!
//! Every setup step shares one layout (see `docs/cli-design.md`, screens
//! 1–5): what is done and what is left, then whatever the current step needs
//! to say or ask. [`render`] draws it from plain data, so each screen can be
//! tested as text; [`Wizard`] puts it on the terminal and reads the keys.

use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chain;
use crate::dashboard::format::wrap;
use crate::dashboard::frame::Frame;

/// How a checklist item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Done, or nothing to do.
    Done,
    /// Being dealt with now.
    Now,
    /// Not reached yet.
    Waiting,
}

/// One line of the checklist.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// What it is: `Node software`.
    pub name: &'static str,
    /// How it stands.
    pub mark: Mark,
    /// What was found, such as the node version.
    pub detail: String,
}

/// A piece of the area under the checklist.
#[derive(Debug, Clone)]
pub enum Part {
    /// A paragraph, wrapped to the screen.
    Text(String),
    /// A line drawn exactly as given.
    Line(Line<'static>),
    /// An empty line.
    Gap,
}

/// Draws a setup screen.
pub fn render(
    title: &str,
    items: &[Item],
    body: &[Part],
    hints: &str,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let mut frame = Frame::new(width);
    let step = items.iter().position(|item| item.mark == Mark::Now).map_or(items.len(), |index| index + 1);
    frame.top(
        vec![Span::raw(format!("{} · {title}", chain::PROGRAM)).bold()],
        vec![Span::raw(format!("{step} of {}", items.len())).dark_gray()],
    );
    frame.blank();
    for item in items {
        let (mark, name) = match item.mark {
            Mark::Done => (Span::raw("✔").green(), Span::raw(format!("  {:<17}", item.name))),
            Mark::Now => (Span::raw("●").yellow(), Span::raw(format!("  {:<17}", item.name)).bold()),
            Mark::Waiting => {
                (Span::raw("○").dark_gray(), Span::raw(format!("  {:<17}", item.name)).dark_gray())
            }
        };
        let mut row = vec![Span::raw("    "), mark, name, Span::raw(item.detail.clone())];
        if item.mark == Mark::Now {
            row.push(Span::raw("   ← now").yellow());
        }
        frame.row(row);
    }
    frame.blank();
    frame.rule(Vec::new());

    let columns = frame.inner().saturating_sub(4).min(76);
    for part in body {
        match part {
            Part::Text(text) => {
                for line in wrap(text, columns) {
                    frame.row(vec![Span::raw(format!("  {line}"))]);
                }
            }
            Part::Line(line) => frame.row(line.spans.clone()),
            Part::Gap => frame.blank(),
        }
    }
    frame.fill_to(height, 1);
    frame.bottom(hints)
}

/// The options of a choice, as body lines, with `selected` marked.
pub fn options(choices: &[String], selected: usize) -> Vec<Part> {
    choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            Part::Line(if index == selected {
                Line::from(vec![
                    Span::raw("    "),
                    Span::raw("▸ ").yellow(),
                    Span::raw(choice.clone()).bold(),
                ])
            } else {
                Line::from(vec![Span::raw(format!("      {choice}"))])
            })
        })
        .collect()
}

/// What a key means to a setup screen.
enum Key {
    /// Quit setup altogether.
    Quit,
    /// Something else.
    Other(KeyEvent),
}

/// The checklist on the terminal, asking its questions.
pub struct Wizard {
    terminal: DefaultTerminal,
    /// `getting set up`, or `needs attention` when only something changed.
    pub title: String,
    /// The checklist.
    pub items: Vec<Item>,
}

/// The answer to a question.
pub enum Answer<T> {
    /// The answer.
    Given(T),
    /// Esc: back to the previous choice.
    Back,
    /// q or Ctrl-C: stop setting up.
    Quit,
}

impl Wizard {
    /// Takes over the terminal.
    pub fn open(title: &str, items: Vec<Item>) -> Self {
        Self { terminal: ratatui::init(), title: title.to_owned(), items }
    }

    /// Gives the terminal back as it was.
    pub fn close(self) {
        drop(self.terminal);
        ratatui::restore();
    }

    /// Sets one item's mark and detail.
    pub fn set(&mut self, name: &str, mark: Mark, detail: impl Into<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.name == name) {
            item.mark = mark;
            item.detail = detail.into();
        }
    }

    /// Draws the screen once.
    pub fn show(&mut self, body: &[Part], hints: &str) -> Result<(), String> {
        let (title, items) = (&self.title, &self.items);
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                let lines =
                    render(title, items, body, hints, usize::from(area.width), usize::from(area.height));
                frame.render_widget(Paragraph::new(lines), area);
            })
            .map(|_| ())
            .map_err(|error| format!("cannot draw: {error}"))
    }

    /// Waits up to `timeout` for a key press.
    fn key(timeout: Duration) -> Result<Option<Key>, String> {
        if !event::poll(timeout).map_err(|error| error.to_string())? {
            return Ok(None);
        }
        match event::read().map_err(|error| error.to_string())? {
            Input::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                Ok(Some(if ctrl_c { Key::Quit } else { Key::Other(key) }))
            }
            _ => Ok(None),
        }
    }

    /// Asks the user to pick one of `choices`, shown after `intro`.
    pub fn choose(&mut self, intro: &[Part], choices: &[String]) -> Result<Answer<usize>, String> {
        let mut selected = 0;
        loop {
            let mut body = intro.to_vec();
            body.push(Part::Gap);
            body.extend(options(choices, selected));
            self.show(&body, "↑↓ choose · enter confirm · q quit")?;
            match Self::key(Duration::from_millis(250))? {
                Some(Key::Quit) => return Ok(Answer::Quit),
                Some(Key::Other(key)) => match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(choices.len() - 1),
                    KeyCode::Enter => return Ok(Answer::Given(selected)),
                    KeyCode::Esc => return Ok(Answer::Back),
                    KeyCode::Char('q') => return Ok(Answer::Quit),
                    _ => {}
                },
                None => {}
            }
        }
    }

    /// Asks for a line of text, checking it with `check` before accepting it.
    pub fn text(
        &mut self,
        intro: &[Part],
        label: &str,
        check: impl Fn(&str) -> Result<(), String>,
    ) -> Result<Answer<String>, String> {
        let mut typed = String::new();
        let mut problem: Option<String> = None;
        loop {
            let mut body = intro.to_vec();
            body.push(Part::Gap);
            body.push(Part::Line(Line::from(vec![
                Span::raw(format!("      {label:<12} ")),
                Span::raw(typed.clone()).bold(),
                Span::raw("▏").yellow(),
            ])));
            if let Some(problem) = &problem {
                body.push(Part::Gap);
                body.push(Part::Line(Line::from(vec![
                    Span::raw("      "),
                    Span::raw(problem.clone()).red(),
                ])));
            }
            self.show(&body, "type or paste · enter confirm · esc back")?;
            match Self::key(Duration::from_millis(250))? {
                Some(Key::Quit) => return Ok(Answer::Quit),
                Some(Key::Other(key)) => match key.code {
                    KeyCode::Enter => match check(typed.trim()) {
                        Ok(()) => return Ok(Answer::Given(typed.trim().to_owned())),
                        Err(why) => problem = Some(why),
                    },
                    KeyCode::Esc => return Ok(Answer::Back),
                    KeyCode::Backspace => {
                        typed.pop();
                        problem = None;
                    }
                    KeyCode::Char(c) => {
                        typed.push(c);
                        problem = None;
                    }
                    _ => {}
                },
                None => {}
            }
        }
    }

    /// Asks for a new passphrase twice, never showing it.
    ///
    /// The characters are held only in this function's memory, shown as dots,
    /// and handed back to the caller to pass straight to the node.
    pub fn passphrase(&mut self, intro: &[Part], after: &[Part]) -> Result<Answer<String>, String> {
        const MINIMUM: usize = 8;
        let mut fields = [String::new(), String::new()];
        let mut focus = 0;
        let mut problem: Option<&str> = None;
        loop {
            let dots = |text: &str| "•".repeat(text.chars().count());
            let cursor =
                |index: usize| if focus == index { Span::raw("▏").yellow() } else { Span::raw("") };
            let matched = !fields[1].is_empty() && fields[0] == fields[1];
            let mut body = intro.to_vec();
            body.push(Part::Gap);
            body.push(Part::Line(Line::from(vec![
                Span::raw(format!("      {:<15}", "Passphrase")),
                Span::raw(dots(&fields[0])),
                cursor(0),
            ])));
            let mut second =
                vec![Span::raw(format!("      {:<15}", "Once more")), Span::raw(dots(&fields[1])), cursor(1)];
            if !fields[1].is_empty() {
                second.push(if matched {
                    Span::raw("   ✔ match").green()
                } else {
                    Span::raw("   ✗ not the same").red()
                });
            }
            body.push(Part::Line(Line::from(second)));
            if let Some(problem) = problem {
                body.push(Part::Gap);
                body.push(Part::Line(Line::from(vec![Span::raw("      "), Span::raw(problem).red()])));
            }
            body.extend_from_slice(after);
            self.show(&body, "enter continue · esc back")?;

            match Self::key(Duration::from_millis(250))? {
                Some(Key::Quit) => return Ok(Answer::Quit),
                Some(Key::Other(key)) => match key.code {
                    KeyCode::Esc => return Ok(Answer::Back),
                    KeyCode::Backspace => {
                        fields[focus].pop();
                        problem = None;
                    }
                    KeyCode::Tab | KeyCode::Down if focus == 0 => focus = 1,
                    KeyCode::BackTab | KeyCode::Up => focus = 0,
                    KeyCode::Enter if focus == 0 => {
                        if fields[0].chars().count() < MINIMUM {
                            problem = Some("use at least 8 characters — longer is better");
                        } else {
                            focus = 1;
                        }
                    }
                    KeyCode::Enter => {
                        if matched {
                            return Ok(Answer::Given(std::mem::take(&mut fields[0])));
                        }
                        problem = Some("the two don't match — try the second one again");
                        fields[1].clear();
                    }
                    KeyCode::Char(c) => {
                        fields[focus].push(c);
                        problem = None;
                    }
                    _ => {}
                },
                None => {}
            }
        }
    }

    /// Shows `body` until enter (true) or q (false).
    pub fn notice(&mut self, body: &[Part], hints: &str) -> Result<bool, String> {
        loop {
            self.show(body, hints)?;
            match Self::key(Duration::from_millis(250))? {
                Some(Key::Quit) => return Ok(false),
                Some(Key::Other(key)) if key.code == KeyCode::Enter => return Ok(true),
                Some(Key::Other(key)) if key.code == KeyCode::Char('q') => return Ok(false),
                _ => {}
            }
        }
    }

    /// Redraws whatever `tick` returns until it says it is done, or the user
    /// quits (false). `tick` runs about four times a second.
    pub fn watch(
        &mut self,
        hints: &str,
        mut tick: impl FnMut() -> Result<Option<Vec<Part>>, String>,
    ) -> Result<bool, String> {
        loop {
            let Some(body) = tick()? else { return Ok(true) };
            self.show(&body, hints)?;
            match Self::key(Duration::from_millis(250))? {
                Some(Key::Quit) => return Ok(false),
                Some(Key::Other(key)) if key.code == KeyCode::Char('q') => return Ok(false),
                _ => {}
            }
        }
    }
}
