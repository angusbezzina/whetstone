//! Help overlay — discovery for every TUI keybinding.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("ESC", "QUIT"),
        ("Q", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lines = vec![
        section("GLOBAL"),
        kv("1–5", "Jump to a screen (Home, Sources, Rules, Violations, Debt)"),
        kv("?", "Open this help screen"),
        kv("Q / ESC", "Quit the TUI"),
        kv("Ctrl-C", "Hard quit"),
        Line::from(""),
        section("CORE CLI"),
        kv("init", "Bootstrap repo + resolve docs"),
        kv("extract", "Draft / submit candidate rules"),
        kv("approve", "Approve candidate rules"),
        kv("actions all", "Generate context + tests + lint"),
        kv("scan", "Scan source files for rule violations"),
        kv("reinit", "Re-resolve changed dependencies/docs"),
        kv("status", "Health + adherence summary"),
        kv("debt", "Deterministic AI-amplified debt hotspots"),
        Line::from(""),
        section("ADVANCED CLI"),
        kv("rules ...", "list | show | add | edit | remove | query | approve | review | worklist"),
        kv("sources ...", "add | edit | list | remove | verify"),
        kv("actions X", "Run just all, context, test, or lint"),
        kv("status --report", "Render the one-page markdown summary"),
        Line::from(""),
        section("IN A LIST"),
        kv("↑ / ↓ / j / k", "Navigate"),
        kv("PgUp / PgDn", "Move faster through longer lists"),
        kv("← / → / h / l", "Horizontal scroll on supported screens"),
        kv("A", "Open add form on Sources or Rules"),
        kv("Tab / Enter / T", "Move fields, save, or toggle Personal/Team while editing"),
        Line::from(""),
        section("SHIPPED SCREENS"),
        kv("Home", "overall health summary with sources, rules, violations, and significant debt"),
        kv("Sources", "internal dependency sources plus handpicked personal and team sources"),
        kv("Rules", "approved rules with file and source detail"),
        kv("Violations", "violation list + config issues"),
        kv("Debt", "ranked hotspot triage for dead/dup/dep/hotspot findings"),
        kv("Result", "generic command-result view for actions without a dedicated screen"),
    ];

    let block = Block::default()
        .title(Span::styled(" HELP ", theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_active());
    let p = Paragraph::new(lines)
        .block(block)
        .scroll((app.help_scroll_y, app.help_scroll_x));
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
