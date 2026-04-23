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
        kv("1–8", "Jump to a screen (Dashboard, Rules, Sources, Extract, Check, Report, Drift, Debt)"),
        kv("R", "Refresh the current screen's data"),
        kv("?", "Toggle this help overlay"),
        kv("Q / ESC", "Quit the TUI"),
        kv("Ctrl-C", "Hard quit"),
        Line::from(""),
        section("CORE CLI"),
        kv("init", "Bootstrap repo + resolve docs"),
        kv("extract", "Draft / submit candidate rules"),
        kv("approve", "Approve candidate rules"),
        kv("actions", "Generate context + tests + lint"),
        kv("check", "Scan source files for rule violations"),
        kv("reinit", "Refresh changed dependencies/docs"),
        kv("status", "Health + adherence summary"),
        kv("debt", "Deterministic AI-amplified debt hotspots"),
        Line::from(""),
        section("ADVANCED CLI"),
        kv("rule ...", "add | edit | query | review | worklist"),
        kv("source ...", "add | list | remove | fetch"),
        kv("actions --only X", "Run just context, tests, or lint"),
        kv("status --report", "Render the one-page markdown summary"),
        Line::from(""),
        section("IN A LIST"),
        kv("↑ / ↓ / j / k", "Navigate"),
        kv("Enter", "Drill into the selected item"),
        kv("/", "Filter"),
        kv("Tab", "Toggle focus between list + detail panes"),
        Line::from(""),
        section("SHIPPED SCREENS"),
        kv("Dashboard", "health, rules, drift, top violations, debt strip"),
        kv("Rules", "approved rules with file and source detail"),
        kv("Sources", "custom source subscriptions across layers"),
        kv("Extract", "worklist + entry detail for extraction"),
        kv("Check", "violation list + config issues"),
        kv("Report", "markdown report viewer"),
        kv("Drift", "dependencies/docs needing re-extraction"),
        kv("Debt", "ranked hotspot triage for dead/dup/dep/hotspot findings"),
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
