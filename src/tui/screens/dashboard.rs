//! Dashboard — a single scrollable repo report card.

use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, DebtView, LanguageCounts},
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
        ("ESC", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let d = &app.dashboard;
    let overall = overall_health_score(app);
    let assessment = assessment_title(app);
    let assessment_body = assessment_description(app);
    let bar_width = area.width.saturating_sub(10) as usize;
    let mut lines: Vec<Line> = vec![
        section("OVERALL HEALTH"),
        kv_line("Overall Health", &score_text(overall), Style::default().fg(theme::AMBER).bold()),
        gauge::render(overall, bar_width),
        Line::from(""),
        section("ASSESSMENT"),
        Line::from(Span::styled(assessment.to_string(), Style::default().fg(theme::AMBER).bold())),
        Line::from(Span::styled(assessment_body, Style::default().fg(theme::MUTED))),
        Line::from(""),
        section("SOURCES"),
        detail_line("Total", &d.sources_total.to_string()),
        detail_line("Internal", &d.sources_internal.to_string()),
        detail_line("External", &d.sources_external.to_string()),
        Line::from(""),
        section("RULES"),
        detail_line("Total", &d.rules_total.to_string()),
    ];

    append_language_lines(&mut lines, &d.rules_by_language, "Rules by language");

    lines.push(Line::from(""));
    lines.push(section("VIOLATIONS"));
    let total_violations = d.violation_counts.must + d.violation_counts.should + d.violation_counts.may;
    lines.push(detail_line("Total", &total_violations.to_string()));
    append_language_count_lines(&mut lines, &d.violations_by_language, "Violations by language");

    if debt_is_significant(app) {
        lines.push(Line::from(""));
        lines.push(section("DEBT"));
        if let DebtView::Ready(summary) = &d.debt {
            lines.push(detail_line(
                "Assessment",
                &theme::humanize_token(&summary.debt_label),
            ));
            lines.push(detail_line("Findings", &summary.finding_count.to_string()));
            for hotspot in summary.hotspots.iter().take(5) {
                lines.push(Line::from(Span::styled(
                    format!("• {} ({})", hotspot.compact_title, hotspot.impact_level),
                    Style::default().fg(theme::MUTED),
                )));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("HOME"))
            .wrap(Wrap { trim: false })
            .scroll((app.dashboard_scroll as u16, 0)),
        area,
    );
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(title.to_string(), theme::header_title()))
}

fn kv_line(label: &str, value: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<18}"), theme::header_meta()),
        Span::styled(value.to_string(), style),
    ])
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    kv_line(label, value, Style::default().fg(ratatui::style::Color::White))
}

fn append_language_lines(lines: &mut Vec<Line<'static>>, by_language: &[(String, usize)], title: &str) {
    lines.push(Line::from(Span::styled(title.to_string(), theme::header_meta())));
    let counts = map_rules_by_language(by_language);
    lines.push(detail_line("TS", &counts.ts.to_string()));
    lines.push(detail_line("Rust", &counts.rust.to_string()));
    lines.push(detail_line("Python", &counts.python.to_string()));
    lines.push(detail_line("All", &counts.all.to_string()));
}

fn append_language_count_lines(lines: &mut Vec<Line<'static>>, counts: &LanguageCounts, title: &str) {
    lines.push(Line::from(Span::styled(title.to_string(), theme::header_meta())));
    lines.push(detail_line("TS", &counts.ts.to_string()));
    lines.push(detail_line("Rust", &counts.rust.to_string()));
    lines.push(detail_line("Python", &counts.python.to_string()));
    lines.push(detail_line("All", &counts.all.to_string()));
}

fn map_rules_by_language(by_language: &[(String, usize)]) -> LanguageCounts {
    let mut counts = LanguageCounts::default();
    for (lang, count) in by_language {
        match lang.as_str() {
            "typescript" => counts.ts += *count,
            "rust" => counts.rust += *count,
            "python" => counts.python += *count,
            _ => counts.all += *count,
        }
    }
    counts
}

fn assessment_title(app: &App) -> &'static str {
    if app.dashboard.rules_total == 0 {
        "Uninitialized"
    } else {
        match overall_health_score(app).unwrap_or(0) {
            85..=100 => "Healthy",
            70..=84 => "Good",
            50..=69 => "Needs Work",
            _ => "At Risk",
        }
    }
}

fn assessment_description(app: &App) -> String {
    if app.dashboard.rules_total == 0 {
        return "Whetstone has not been initialized with an approved ruleset for this repo yet. Start by initializing or authoring the first rules.".to_string();
    }

    let total_violations = app.dashboard.violation_counts.must
        + app.dashboard.violation_counts.should
        + app.dashboard.violation_counts.may;
    let debt = match &app.dashboard.debt {
        DebtView::Ready(summary) => summary.finding_count,
        _ => 0,
    };

    if total_violations == 0 && debt == 0 {
        "The repo is largely clean right now: there are no current violations and no material debt hotspots surfaced by Whetstone.".to_string()
    } else if total_violations > 0 {
        format!(
            "Whetstone found {total_violations} active violations. Review those first, then revisit any broader debt hotspots that remain."
        )
    } else {
        format!(
            "The ruleset is active and current, but Whetstone still sees {debt} debt hotspot(s) worth triaging."
        )
    }
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

fn score_text(score: Option<i64>) -> String {
    score
        .map(|s| format!("{s}/100"))
        .unwrap_or_else(|| "N/A".to_string())
}

fn panel_block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive())
}

fn debt_is_significant(app: &App) -> bool {
    matches!(
        &app.dashboard.debt,
        DebtView::Ready(summary)
            if matches!(summary.debt_label.as_str(), "high" | "elevated")
                || summary.finding_count >= 20
    )
}
