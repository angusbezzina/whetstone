//! Help overlay — discovery for every TUI keybinding.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{components::footer, theme};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("ESC", "BACK"),
        ("Q", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: Rect) {
    let lines = vec![
        section("GLOBAL"),
        kv("1–7", "Jump to a screen (Dashboard, Rules, Sources, Extract, Check, Report, Drift)"),
        kv("R", "Refresh the current screen's data"),
        kv("?", "Toggle this help overlay"),
        kv("Q / ESC", "Quit the TUI"),
        kv("Ctrl-C", "Hard quit"),
        Line::from(""),
        section("IN A LIST"),
        kv("↑ / ↓ / j / k", "Navigate"),
        kv("Enter", "Drill into the selected item"),
        kv("/", "Filter"),
        kv("Tab", "Toggle focus between list + detail panes"),
        Line::from(""),
        section("SHIPPED SCREENS (v0.4.0)"),
        kv("Dashboard", "rule_system_score · adherence_score · drift · top violations"),
        Line::from(""),
        section("COMING SOON"),
        kv("Rules, Sources, Extract, Check, Report, Drift", "placeholder stubs today — implementation in follow-up beads"),
    ];

    let block = Block::default()
        .title(Span::styled(" HELP ", theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_active());
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn section(label: &str) -> Line<'static> {
    Line::from(Span::styled(label.to_string(), theme::header_title()))
}

fn kv(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<14} ", key), theme::key_hint_accent()),
        Span::styled(desc.to_string(), Style::default().fg(ratatui::style::Color::White)),
    ])
}
