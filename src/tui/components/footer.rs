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
    ("2", "INTERNAL SOURCES"),
    ("3", "EXTERNAL SOURCES"),
    ("4", "RULES"),
    ("5", "VIOLATIONS"),
    ("6", "DEBT"),
    ("?", "HELP"),
    ("ESC", "QUIT"),
    ("Q", "QUIT"),
];

pub fn global_hints() -> &'static [Hint] {
    FULL_HINTS
}

pub fn render(frame: &mut Frame<'_>, area: Rect, hints: &[Hint]) {
    let mut spans: Vec<Span> = Vec::with_capacity(hints.len() * 3);
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default()));
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
