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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollHint {
    pub up: bool,
    pub down: bool,
}

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

pub fn render(frame: &mut Frame<'_>, area: Rect, hints: &[Hint], scroll_hint: Option<ScrollHint>) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme::AMBER));
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    if inner.height == 0 {
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ])
        .split(inner);

    let left = Paragraph::new(Line::from(render_menu_spans(hints)));
    let middle = Paragraph::new(Line::from(render_scroll_spans(scroll_hint)));
    let right = Paragraph::new(Line::from(vec![
        Span::styled("?", theme::key_hint_accent()),
        Span::styled(": HELP, ", theme::key_hint_label()),
        Span::styled("ESC", theme::key_hint_accent()),
        Span::styled(": Quit", theme::key_hint_label()),
    ]))
    .right_aligned();

    frame.render_widget(left, cols[0]);
    frame.render_widget(middle, cols[1]);
    frame.render_widget(right, cols[2]);
}

pub fn render_form(frame: &mut Frame<'_>, area: Rect, hints: &[Hint]) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme::AMBER));
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    if inner.height == 0 {
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ])
        .split(inner);

    let left = Paragraph::new(Line::from(render_menu_spans(hints)));
    let middle = Paragraph::new(Line::from(vec![
        Span::styled("ENTER", theme::key_hint_accent()),
        Span::styled(": Submit", theme::key_hint_label()),
    ]));
    let right = Paragraph::new(Line::from(vec![
        Span::styled("ESC", theme::key_hint_accent()),
        Span::styled(": Cancel", theme::key_hint_label()),
    ]))
    .right_aligned();

    frame.render_widget(left, cols[0]);
    frame.render_widget(middle, cols[1]);
    frame.render_widget(right, cols[2]);
}

fn render_scroll_spans(scroll_hint: Option<ScrollHint>) -> Vec<Span<'static>> {
    let Some(scroll_hint) = scroll_hint else {
        return vec![Span::raw("")];
    };

    let mut spans = Vec::new();
    if scroll_hint.up {
        spans.push(Span::styled("↑", theme::key_hint_accent()));
        spans.push(Span::styled(" Up", Style::default().fg(theme::MUTED)));
    }
    if scroll_hint.up && scroll_hint.down {
        spans.push(Span::raw("  "));
    }
    if scroll_hint.down {
        spans.push(Span::styled("↓", theme::key_hint_accent()));
        spans.push(Span::styled(" Down", Style::default().fg(theme::MUTED)));
    }
    spans
}

fn render_menu_spans(hints: &[Hint]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, (key, label)) in hints.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(
                ", ".to_string(),
                Style::default().fg(theme::MUTED),
            ));
        }
        spans.push(Span::styled((*key).to_string(), theme::key_hint_accent()));
        spans.push(Span::styled(format!(" {label}"), theme::key_hint_label()));
    }
    spans
}
