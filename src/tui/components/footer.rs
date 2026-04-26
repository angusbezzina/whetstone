//! Footer key-hint bar.
//!
//! Keys render in bold amber; labels in dim white, ALL-CAPS.
//! Hints are space-separated and wrap cleanly on narrow terminals.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::theme;

/// A single key hint: `(key, label)`. Label should already be uppercase.
pub type Hint = (&'static str, &'static str);

const FULL_HINTS: &[Hint] = &[
    ("1", "HOME"),
    ("2", "RULES"),
    ("3", "SOURCES"),
    ("4", "EXTRACT"),
    ("5", "CHECK"),
    ("6", "REPORT"),
    ("7", "DRIFT"),
    ("8", "DEBT"),
    ("R", "REFRESH"),
    ("?", "HELP"),
    ("Q", "QUIT"),
];

const COMPACT_HINTS: &[Hint] = &[("1", "HOME"), ("R", "REFRESH"), ("?", "HELP"), ("Q", "QUIT")];

fn hint_width(hints: &[Hint]) -> usize {
    hints
        .iter()
        .enumerate()
        .map(|(i, (key, label))| {
            let sep = if i == 0 { 0 } else { 2 };
            sep + key.len() + 1 + label.len()
        })
        .sum()
}

fn hints_for_width(width: u16) -> &'static [Hint] {
    let usable = width.saturating_sub(2) as usize;
    if hint_width(FULL_HINTS) <= usable {
        FULL_HINTS
    } else {
        COMPACT_HINTS
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, _hints: &[Hint]) {
    let hints = hints_for_width(area.width);
    let mut spans: Vec<Span> = Vec::with_capacity(hints.len() * 3);
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(*key, theme::key_hint_accent()));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(*label, theme::key_hint_label()));
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme::AMBER));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}
