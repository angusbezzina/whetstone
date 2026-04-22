//! "Coming soon" placeholder for screens not yet implemented (Rules, Sources,
//! Extract, Check, Report, Drift). All share this render path until they ship
//! in their own follow-up beads under Epic 4A.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{components::footer, msg::Screen, theme};

pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, screen: Screen) {
    let title = screen.title();
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {title}"),
            theme::header_title(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Coming soon — this screen is scheduled for a follow-up bead under",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            "  Epic 4A (TUI).",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Until then, the CLI commands behind this screen still work:",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            format!("    wh {}", cli_hint(screen)),
            Style::default().fg(theme::AMBER),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [1] to return to the Dashboard.",
            Style::default().fg(theme::MUTED),
        )),
    ];

    let block = Block::default()
        .title(Span::styled(
            format!(" {title} "),
            theme::header_title(),
        ))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn cli_hint(screen: Screen) -> &'static str {
    match screen {
        Screen::Rules => "review [--status approved|candidate]",
        Screen::Sources => "source list",
        Screen::Extract => "extract",
        Screen::Check => "check src/",
        Screen::Report => "report",
        Screen::Drift => "reinit",
        Screen::Debt => "debt",
        Screen::Dashboard | Screen::Help => "tui",
    }
}
