//! Footer key-hint bar.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::theme;

pub type Hint = (&'static str, &'static str);

const MENU_HINTS: &[Hint] = &[
    ("1", "HOME"),
    ("2", "SOURCES"),
    ("3", "RULES"),
    ("4", "VIOLATIONS"),
    ("5", "DEBT"),
];

pub fn global_hints() -> &'static [Hint] {
    MENU_HINTS
}

pub fn render(frame: &mut Frame<'_>, area: Rect, hints: &[Hint], scroll_hint: Option<&str>) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ])
        .split(area);

    let left = Paragraph::new(Line::from(render_menu_spans(hints)));
    let middle = Paragraph::new(Line::from(if let Some(text) = scroll_hint {
        vec![Span::styled(
            text.to_string(),
            Style::default().fg(theme::MUTED),
        )]
    } else {
        vec![Span::raw("")]
    }));
    let right = Paragraph::new(Line::from(vec![
        Span::styled("?", theme::key_hint_accent()),
        Span::styled(": HELP, ", theme::key_hint_label()),
        Span::styled("ESC", theme::key_hint_accent()),
        Span::styled(": Quit", theme::key_hint_label()),
    ]))
    .right_aligned();

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme::AMBER));
    frame.render_widget(block, area);
    frame.render_widget(left, cols[0]);
    frame.render_widget(middle, cols[1]);
    frame.render_widget(right, cols[2]);
}

fn render_menu_spans(hints: &[Hint]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, (key, label)) in hints.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(", ".to_string(), Style::default().fg(theme::MUTED)));
        }
        spans.push(Span::styled((*key).to_string(), theme::key_hint_accent()));
        spans.push(Span::styled(format!(" {label}"), theme::key_hint_label()));
    }
    spans
}
