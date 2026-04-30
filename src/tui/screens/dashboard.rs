//! Dashboard — compact landing summary for health, sources, rules,
//! violations, and materially significant debt.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, DebtView},
    components::{footer, gauge},
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("2", "SOURCES"),
        ("3", "RULES"),
        ("4", "VIOLATIONS"),
        ("5", "DEBT"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width < 80 || area.height < 20 {
        render_compact(frame, area, app);
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(area);

    let bottom = if debt_is_significant(app) {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(outer[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(outer[1])
    };

    render_overall_health(frame, outer[0], app);
    render_sources_panel(frame, bottom[0], app);
    render_rules_panel(frame, bottom[1], app);
    render_violations_panel(frame, bottom[2], app);
    if debt_is_significant(app) {
        render_debt_panel(frame, bottom[3], app);
    }
}

fn render_overall_health(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let overall = overall_health_score(app);
    let label = overall_health_label(overall);
    let bar_width = (area.width as usize).saturating_sub(28).clamp(12, 42);

    let lines = vec![
        Line::from(vec![
            Span::styled("Overall Health  ", theme::header_meta()),
            Span::styled(
                overall
                    .map(|s| format!("{s} / 100"))
                    .unwrap_or_else(|| "N/A".to_string()),
                Style::default().fg(theme::AMBER).bold(),
            ),
            Span::raw("   "),
            Span::styled(label, Style::default().fg(theme::AMBER).bold()),
        ]),
        gauge::render(overall, bar_width),
        Line::from(""),
        Line::from(Span::styled(
            "Calculated as the average of Rule System and Adherence when both are available. If Adherence is unavailable, Whetstone falls back to Rule System only.",
            Style::default().fg(theme::MUTED),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("OVERALL HEALTH"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_sources_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let lines = vec![
        pair_line("Total", &d.sources_total.to_string()),
        Line::from(""),
        Line::from(Span::styled(
            if d.sources_total == 0 {
                "No sources discovered yet. Run wh init to populate dependency sources or add a handpicked source."
            } else {
                "Includes dependency sources plus handpicked personal/team sources."
            },
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("SOURCES"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_rules_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let lines = vec![pair_line("Total", &d.rules_total.to_string())];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("RULES"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_violations_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let total = d.violation_counts.must + d.violation_counts.should + d.violation_counts.may;
    let lines = vec![pair_line("Total", &total.to_string())];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("VIOLATIONS"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_debt_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lines: Vec<Line<'static>> = match &app.dashboard.debt {
        DebtView::NotComputed => vec![
            Line::from(Span::styled(
                "Debt has not been computed yet.",
                Style::default().fg(theme::MUTED),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Press ", Style::default().fg(theme::MUTED)),
                Span::styled("5", theme::header_meta()),
                Span::raw(" to open the debt screen."),
            ]),
        ],
        DebtView::Loading => vec![Line::from(Span::styled(
            "Computing debt…",
            Style::default().fg(theme::MUTED),
        ))],
        DebtView::Error(msg) => vec![
            Line::from(Span::styled(
                "Debt compute failed:",
                Style::default().fg(theme::STATUS_WARN),
            )),
            Line::from(truncate(msg, area.width.saturating_sub(4) as usize)),
        ],
        DebtView::Ready(summary) => vec![
            Line::from(vec![
                Span::styled("Debt Label  ", theme::header_meta()),
                Span::styled(
                    theme::humanize_token(&summary.debt_label),
                    Style::default()
                        .fg(theme::debt_label_color(&summary.debt_label))
                        .bold(),
                ),
            ]),
            Line::from(vec![
                pair_span("Total", &summary.finding_count.to_string()),
                Span::raw("   ·   "),
                pair_span("Dead", &summary.by_dead.to_string()),
                Span::raw("   ·   "),
                pair_span("Dup", &summary.by_dup.to_string()),
                Span::raw("   ·   "),
                pair_span("Deps", &summary.by_deps.to_string()),
            ]),
        ],
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("DEBT"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_compact(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let overall = overall_health_score(app);
    let bar_width = (area.width as usize).saturating_sub(20).clamp(6, 24);
    let total_violations = d.violation_counts.must + d.violation_counts.should + d.violation_counts.may;

    let mut lines = vec![gauge_row("Overall Health", gauge::render(overall, bar_width))];
    lines.push(Line::from(Span::styled(
        "Average of Rule System and Adherence when both are available.",
        Style::default().fg(theme::MUTED),
    )));
    lines.push(Line::from(""));
    lines.push(pair_line("Sources", &d.sources_total.to_string()));
    lines.push(pair_line("Rules", &d.rules_total.to_string()));
    lines.push(pair_line("Violations", &total_violations.to_string()));
    if debt_is_significant(app) {
        if let DebtView::Ready(summary) = &d.debt {
            lines.push(pair_line(
                "Debt",
                &format!("{} ({})", theme::humanize_token(&summary.debt_label), summary.finding_count),
            ));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("HOME"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn overall_health_score(app: &App) -> Option<i64> {
    let d = &app.dashboard;
    match (d.rule_system_score, d.adherence_score) {
        (Some(rule), Some(adherence)) => Some(((rule + adherence) / 2).clamp(0, 100)),
        (Some(rule), None) => Some(rule.clamp(0, 100)),
        (None, Some(adherence)) => Some(adherence.clamp(0, 100)),
        (None, None) => None,
    }
}

fn overall_health_label(score: Option<i64>) -> &'static str {
    match score.unwrap_or(0) {
        85..=100 => "Healthy",
        70..=84 => "Good",
        50..=69 => "Needs Work",
        _ => "At Risk",
    }
}

fn gauge_row(label: &'static str, gauge_line: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{label} "), theme::header_meta())];
    spans.extend(gauge_line.spans);
    Line::from(spans)
}

fn pair_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), theme::header_meta()),
        Span::raw(value.to_string()),
    ])
}

fn pair_span(label: &str, value: &str) -> Span<'static> {
    Span::raw(format!("{label} {value}"))
}

fn panel_block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn debt_is_significant(app: &App) -> bool {
    matches!(
        &app.dashboard.debt,
        DebtView::Ready(summary)
            if matches!(summary.debt_label.as_str(), "high" | "elevated")
                || summary.finding_count >= 20
    )
}
