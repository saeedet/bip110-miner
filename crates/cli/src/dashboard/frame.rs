//! A box of text lines with ruled sections, drawn to an exact width.
//!
//! The screens are designed as text (see `docs/cli-design.md`), so they are
//! built as text: one [`Line`] per terminal row, borders included. That keeps
//! what is drawn identical to what was designed, and lets tests compare a
//! whole screen as a string.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// The style of borders and rules: present, but quiet.
fn border() -> Style {
    Style::new().dark_gray()
}

/// A screen under construction.
pub struct Frame {
    width: usize,
    lines: Vec<Line<'static>>,
}

impl Frame {
    /// An empty screen `width` columns wide.
    pub fn new(width: usize) -> Self {
        Self { width: width.max(20), lines: Vec::new() }
    }

    /// Columns between the side borders.
    pub fn inner(&self) -> usize {
        self.width - 2
    }

    /// Rows drawn so far.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `╭─ title ──────── right ─╮`
    pub fn top(&mut self, title: Vec<Span<'static>>, right: Vec<Span<'static>>) {
        // "╭─ " title " " dashes [" " right " "] "─╮"
        let right_part = if right.is_empty() { 0 } else { width(&right) + 2 };
        let dashes = self.width.saturating_sub(3 + width(&title) + 1 + right_part + 2);
        let mut spans = vec![Span::styled("╭─ ", border())];
        spans.extend(title);
        spans.push(Span::styled(format!(" {}", "─".repeat(dashes)), border()));
        if !right.is_empty() {
            spans.push(Span::raw(" "));
            spans.extend(right);
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled("─╮", border()));
        self.lines.push(fit(spans, self.width));
    }

    /// `│ content            │`
    pub fn row(&mut self, content: Vec<Span<'static>>) {
        let mut spans = vec![Span::styled("│", border())];
        spans.extend(pad(content, self.inner()).spans);
        spans.push(Span::styled("│", border()));
        self.lines.push(Line::from(spans));
    }

    /// An empty row.
    pub fn blank(&mut self) {
        self.row(Vec::new());
    }

    /// `├─ LABEL ─────────────┤`, or a plain rule without a label.
    pub fn rule(&mut self, label: Vec<Span<'static>>) {
        self.lines.push(self.ruled('├', label, '┤', None));
    }

    /// A rule that opens two columns: `├─ LEFT ───┬─ RIGHT ───┤`.
    pub fn split(&mut self, left: Vec<Span<'static>>, right: Vec<Span<'static>>, left_width: usize) {
        let mut spans = self.ruled('├', left, '┬', Some(left_width)).spans;
        let rest = self.inner() - left_width - 1;
        spans.extend(ruled_part(right, rest));
        spans.push(Span::styled("┤", border()));
        self.lines.push(Line::from(spans));
    }

    /// A row in two columns.
    pub fn columns(&mut self, left: Vec<Span<'static>>, right: Vec<Span<'static>>, left_width: usize) {
        let mut spans = vec![Span::styled("│", border())];
        spans.extend(pad(left, left_width).spans);
        spans.push(Span::styled("│", border()));
        spans.extend(pad(right, self.inner() - left_width - 1).spans);
        spans.push(Span::styled("│", border()));
        self.lines.push(Line::from(spans));
    }

    /// A rule that closes two columns: `├─ LABEL ─────┴──────┤`.
    pub fn join(&mut self, label: Vec<Span<'static>>, left_width: usize) {
        let mut spans = self.ruled('├', label, '┴', Some(left_width)).spans;
        let rest = self.inner() - left_width - 1;
        spans.push(Span::styled("─".repeat(rest), border()));
        spans.push(Span::styled("┤", border()));
        self.lines.push(Line::from(spans));
    }

    /// Blank rows until `rows` rows are left for whatever follows.
    pub fn fill_to(&mut self, height: usize, rows_after: usize) {
        while self.len() + rows_after < height {
            self.blank();
        }
    }

    /// `╰─ q quit · ? help ─────╯`, and the finished screen.
    pub fn bottom(mut self, hints: &str) -> Vec<Line<'static>> {
        let spans = vec![
            Span::styled("╰─ ", border()),
            Span::raw(hints.to_owned()),
            Span::styled(
                format!(" {}╯", "─".repeat(self.width.saturating_sub(5 + hints.chars().count()))),
                border(),
            ),
        ];
        self.lines.push(fit(spans, self.width));
        self.lines
    }

    /// `left` + label + dashes, `inner` columns wide when given, then `end`.
    fn ruled(&self, left: char, label: Vec<Span<'static>>, end: char, inner: Option<usize>) -> Line<'static> {
        let inner = inner.unwrap_or(self.inner());
        let mut spans = vec![Span::styled(left.to_string(), border())];
        spans.extend(ruled_part(label, inner));
        spans.push(Span::styled(end.to_string(), border()));
        Line::from(spans)
    }
}

/// `─ LABEL ─────` exactly `columns` wide, or all dashes without a label.
fn ruled_part(label: Vec<Span<'static>>, columns: usize) -> Vec<Span<'static>> {
    if label.is_empty() {
        return vec![Span::styled("─".repeat(columns), border())];
    }
    // Cut to fit if need be, then drop the padding that came with it.
    let label = trim_trailing_spaces(pad(label, columns.saturating_sub(4)).spans);
    let dashes = columns.saturating_sub(2 + width(&label) + 1);
    let mut spans = vec![Span::styled("─ ", border())];
    spans.extend(label);
    spans.push(Span::styled(format!(" {}", "─".repeat(dashes)), border()));
    spans
}

/// Drops padding added by [`pad`], so a rule's dashes start after the label.
fn trim_trailing_spaces(mut spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    while let Some(last) = spans.last_mut() {
        let trimmed = last.content.trim_end().to_owned();
        if trimmed.is_empty() {
            spans.pop();
        } else {
            last.content = trimmed.into();
            break;
        }
    }
    spans
}

/// Total display width of some spans.
pub fn width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Pads with spaces, or cuts with `…`, to exactly `columns`.
pub fn pad(spans: Vec<Span<'static>>, columns: usize) -> Line<'static> {
    let total = width(&spans);
    if total <= columns {
        let mut spans = spans;
        spans.push(Span::raw(" ".repeat(columns - total)));
        return Line::from(spans);
    }
    let mut out = Vec::new();
    let mut left = columns.saturating_sub(1);
    for span in spans {
        if left == 0 {
            break;
        }
        let text: String = span.content.chars().take(left).collect();
        left -= text.chars().count();
        out.push(Span::styled(text, span.style));
    }
    out.push(Span::raw("…"));
    Line::from(out)
}

/// Like [`pad`], for a whole border line that must not be cut mid-corner.
fn fit(spans: Vec<Span<'static>>, columns: usize) -> Line<'static> {
    if width(&spans) <= columns { Line::from(spans) } else { pad(spans, columns) }
}

/// A screen's plain text, one row per line, for tests.
#[cfg(test)]
pub fn text(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_exactly_the_screen_width() {
        let mut frame = Frame::new(40);
        frame.top(vec!["title".into()], vec!["right".into()]);
        frame.row(vec!["hello".into()]);
        frame.row(vec!["x".repeat(60).into()]);
        frame.rule(vec!["LABEL".into()]);
        frame.split(vec!["L".into()], vec!["R".into()], 18);
        frame.columns(vec!["a".into()], vec!["b".into()], 18);
        frame.join(vec!["J".into()], 18);
        let lines = frame.bottom("q quit");
        for line in text(&lines).lines() {
            assert_eq!(line.chars().count(), 40, "{line:?}");
        }
    }

    #[test]
    fn rules_look_like_the_design() {
        let mut frame = Frame::new(24);
        frame.rule(vec!["MINING".into()]);
        frame.split(vec!["A".into()], vec!["B".into()], 10);
        frame.join(vec!["C".into()], 10);
        assert_eq!(
            text(&frame.bottom("q")),
            "├─ MINING ─────────────┤\n├─ A ──────┬─ B ───────┤\n├─ C ──────┴───────────┤\n╰─ q ──────────────────╯"
        );
    }
}
