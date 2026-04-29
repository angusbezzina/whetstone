//! Help overlay — discovery for every TUI keybinding.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("ESC", "BACK"),
        ("Q", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
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
        kv("actions all", "Generate context + tests + lint"),
        kv("scan", "Scan source files for rule violations"),
        kv("reinit", "Refresh changed dependencies/docs"),
        kv("status", "Health + adherence summary"),
        kv("debt", "Deterministic AI-amplified debt hotspots"),
        Line::from(""),
        section("ADVANCED CLI"),
        kv("rules ...", "list | show | add | edit | remove | query | approve | worklist"),
        kv("sources ...", "add | edit | list | remove | verify"),
        kv("actions X", "Run just all, context, test, or lint"),
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
