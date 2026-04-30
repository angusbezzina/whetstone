//! Top bar: `WHETSTONE › <screen>   project                v0.x.y`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::theme;

pub fn render(frame: &mut Frame<'_>, area: Rect, breadcrumb: &str, project_path: &str) {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(28),
            Constraint::Min(0),
            Constraint::Length(version.len() as u16 + 2),
        ])
        .split(area);

    let left = Paragraph::new(Line::from(vec![
        Span::styled("WHETSTONE ", theme::header_title()),
        Span::styled(format!("› {breadcrumb}"), theme::header_title()),
    ]));

    let center = Paragraph::new(Line::from(Span::styled(
        truncate_middle(project_path, cols[1].width.saturating_sub(1) as usize),
        theme::header_meta(),
    )));

    let right = Paragraph::new(Line::from(Span::styled(version, theme::header_title())));

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme::AMBER));

    frame.render_widget(block, area);
    frame.render_widget(left, cols[0]);
    frame.render_widget(center, cols[1]);
    frame.render_widget(right, cols[2]);
}

fn truncate_middle(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max || max <= 1 {
        return s.to_string();
    }
    if max <= 3 {
        return s.chars().take(max).collect();
    }
    let left = (max - 1) / 2;
    let right = max - 1 - left;
    let prefix: String = s.chars().take(left).collect();
    let suffix: String = s
        .chars()
        .rev()
        .take(right)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}
