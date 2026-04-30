//! Dashboard — landing screen. Presents Whetstone as a living report card
//! with a single overall health score and supporting domains: rules,
//! violations, reinit status, and debt.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, DebtView, ViolationCounts},
    components::{footer, gauge},
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("2", "INTERNAL SOURCES"),
        ("3", "EXTERNAL SOURCES"),
        ("4", "RULES"),
        ("5", "VIOLATIONS"),
        ("6", "DEBT"),
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
        .constraints([Constraint::Length(8), Constraint::Min(10)])
        .split(area);

    let lower_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[1]);

    let top_breakdown = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(lower_rows[0]);

    let bottom_breakdown = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(lower_rows[1]);

    render_overall_health(frame, outer[0], app);
    render_rules_panel(frame, top_breakdown[0], app);
    render_violations_panel(frame, top_breakdown[1], app);
    render_reinit_panel(frame, bottom_breakdown[0], app);
    render_debt_panel(frame, bottom_breakdown[1], app);
}

fn render_overall_health(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let overall = overall_health_score(app);
    let label = overall_health_label(overall);
    let bar_width = (area.width as usize).saturating_sub(28).clamp(12, 42);

    let mut lines = vec![
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
        Line::from(vec![
            pair_span("Rule System", &score_text(d.rule_system_score)),
            Span::raw("   ·   "),
            pair_span("Adherence", &score_text(d.adherence_score)),
        ]),
    ];

    if d.drift_count > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "{} dependency or docs changes need attention. Run `wh reinit` to refresh sources and review rule freshness.",
                d.drift_count
            ),
            Style::default().fg(theme::AMBER),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("OVERALL HEALTH"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_rules_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let mut lines = vec![
        pair_line("Approved", &d.rules_total.to_string()),
        pair_line("Personal", &d.rules_personal.to_string()),
        Line::from(""),
    ];

    if d.rules_by_language.is_empty() {
        lines.push(Line::from(Span::styled(
            "No approved rules loaded yet.",
            Style::default().fg(theme::MUTED),
        )));
    } else {
        lines.push(Line::from(Span::styled("Coverage", theme::header_meta())));
        for (lang, n) in &d.rules_by_language {
            lines.push(Line::from(format!(
                "  {}  {}",
                theme::humanize_token(lang),
                n
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines).block(panel_block("RULES")).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_violations_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let total = d.violation_counts.must + d.violation_counts.should + d.violation_counts.may;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Total  ", theme::header_meta()),
            Span::styled(total.to_string(), Style::default().fg(theme::AMBER).bold()),
        ]),
        violations_line(&d.violation_counts),
        Line::from(""),
    ];

    if d.top_violations.is_empty() {
        lines.push(Line::from(Span::styled(
            "No violations detected.",
            Style::default().fg(ratatui::style::Color::White),
        )));
    } else {
        lines.push(Line::from(Span::styled("Top Issues", theme::header_meta())));
        for v in d.top_violations.iter().skip(app.dashboard_scroll).take(3) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  ", theme::humanize_token(&v.severity)),
                    Style::default().fg(theme::severity_color(&v.severity)).bold(),
                ),
                Span::styled(truncate(&v.rule_id, 26), Style::default().fg(theme::AMBER)),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  {}:{}", v.file, v.line),
                Style::default().fg(theme::MUTED),
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("VIOLATIONS"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_reinit_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let mut lines = vec![
        pair_line("Changed Dependencies", &d.drift_count.to_string()),
        pair_line(
            "Last Reinit",
            d.last_refresh
                .as_deref()
                .map(|last| last.split('T').next().unwrap_or(last))
                .unwrap_or("Never"),
        ),
        Line::from(""),
    ];

    if d.drift_deps.is_empty() && d.drift_count == 0 {
        lines.push(Line::from(Span::styled(
            "No drift detected. No reinit needed right now.",
            Style::default().fg(ratatui::style::Color::White),
        )));
    } else {
        lines.push(Line::from(Span::styled("Next Action", theme::header_meta())));
        lines.push(Line::from(Span::styled(
            "Run `wh reinit` to refresh dependency docs and recompute freshness.",
            Style::default().fg(theme::AMBER),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Changed Packages", theme::header_meta())));
        for dep in d.drift_deps.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled("  ▸ ", Style::default().fg(theme::AMBER)),
                Span::raw(dep.clone()),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("REINIT STATUS"))
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
                Span::styled("6", theme::header_meta()),
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
        DebtView::Ready(summary) => {
            let mut v = vec![
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
                    Span::raw("   ·   "),
                    pair_span("Hot", &summary.by_hotspots.to_string()),
                ]),
                Line::from(""),
            ];

            if summary.hotspots.is_empty() {
                v.push(Line::from(Span::styled(
                    "No hotspots. Nothing to triage.",
                    Style::default().fg(ratatui::style::Color::White),
                )));
            } else {
                for h in summary.hotspots.iter().skip(app.dashboard_scroll).take(2) {
                    v.push(Line::from(vec![
                        Span::styled(
                            format!("  {}  ", theme::humanize_token(&h.category)),
                            Style::default().fg(theme::AMBER).bold(),
                        ),
                        Span::raw(truncate(&h.title, area.width.saturating_sub(18) as usize)),
                    ]));
                }
            }
            v
        }
    };

    frame.render_widget(
        Paragraph::new(lines).block(panel_block("DEBT")).wrap(Wrap { trim: false }),
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
    lines.push(pair_line("Rules", &format!("{} approved", d.rules_total)));
    lines.push(pair_line("Violations", &total_violations.to_string()));
    lines.push(pair_line("Reinit", &format!("{} deps changed", d.drift_count)));
    match &d.debt {
        DebtView::Ready(summary) => {
            lines.push(pair_line(
                "Debt",
                &format!("{} ({})", theme::humanize_token(&summary.debt_label), summary.finding_count),
            ));
        }
        _ => lines.push(pair_line("Debt", "Not computed")),
    }

    frame.render_widget(
        Paragraph::new(lines).block(panel_block("DASHBOARD")).wrap(Wrap { trim: false }),
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

fn score_text(score: Option<i64>) -> String {
    score
        .map(|s| format!("{s} / 100"))
        .unwrap_or_else(|| "N/A".to_string())
}

fn violations_line(c: &ViolationCounts) -> Line<'static> {
    Line::from(vec![
        Span::styled("●", Style::default().fg(theme::SEVERITY_MUST)),
        Span::raw(format!(" Must {:<3} ", c.must)),
        Span::styled("●", Style::default().fg(theme::SEVERITY_SHOULD)),
        Span::raw(format!(" Should {:<3} ", c.should)),
        Span::styled("●", Style::default().fg(theme::SEVERITY_MAY)),
        Span::raw(format!(" May {}", c.may)),
    ])
}

fn gauge_row(label: &'static str, gauge_line: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{label} "), theme::header_meta())];
    spans.extend(gauge_line.spans);
    Line::from(spans)
}

fn pair_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<20}"), theme::header_meta()),
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
