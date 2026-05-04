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
    ("2", "SOURCES"),
    ("3", "RULES"),
    ("4", "VIOLATIONS"),
    ("5", "DEBT"),
    ("?", "HELP"),
    ("ESC", "QUIT"),
];

pub fn global_hints() -> &'static [Hint] {
    FULL_HINTS
}

pub fn render(frame: &mut Frame<'_>, area: Rect, hints: &[Hint], show_scroll: bool) {
    let nav_text = render_hints_text(&hints[..hints.len().min(5)]);
    let right_text = render_hints_text(&hints[hints.len().saturating_sub(2)..]);
    let scroll_text = if show_scroll {
        "  Scroll Up Scroll Down"
    } else {
        ""
    };
    let left_text = format!("{nav_text}{scroll_text}");

    let total_width = area.width.saturating_sub(2) as usize;
    let spacer_len = total_width
        .saturating_sub(left_text.chars().count())
        .saturating_sub(right_text.chars().count());
    let spacer = " ".repeat(spacer_len.max(1));

    let spans: Vec<Span> = vec![
        Span::raw(left_text),
        Span::raw(spacer),
        Span::raw(right_text),
    ];

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme::AMBER));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}

fn render_hints_text(hints: &[Hint]) -> String {
    hints
        .iter()
        .map(|(key, label)| format!("{key}: {label}"))
        .collect::<Vec<_>>()
        .join(", ")
}
