//! Dashboard — landing screen. Shows health + rules + drift at a glance
//! plus the top violations pulled from `wh check`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{
    app::{App, ViolationCounts},
    components::{footer, gauge},
    theme,
};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("2", "RULES"),
        ("3", "SOURCES"),
        ("4", "EXTRACT"),
        ("5", "CHECK"),
        ("6", "REPORT"),
        ("R", "REFRESH"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    // Below the minimum, show the compact layout (no side-by-side panels).
    if area.width < 80 || area.height < 20 {
        render_compact(frame, area, app);
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),   // top row (health | rules | drift)
            Constraint::Min(8),      // top violations (flexible)
            Constraint::Length(1),   // spacer
        ])
        .split(area);

    let top_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(outer[0]);

    render_health_panel(frame, top_row[0], app);
    render_rules_panel(frame, top_row[1], app);
    render_drift_panel(frame, top_row[2], app);
    render_violations_panel(frame, outer[1], app);
}

fn render_health_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let bar_width = (area.width as usize).saturating_sub(20).clamp(10, 32);

    let must = d.violation_counts.must;
    let should = d.violation_counts.should;
    let may = d.violation_counts.may;

    let lines = vec![
        Line::from(Span::styled("Rule system", theme::header_meta())),
        gauge::render(d.rule_system_score, bar_width),
        Line::from(""),
        Line::from(Span::styled("Adherence", theme::header_meta())),
        gauge::render(d.adherence_score, bar_width),
        Line::from(""),
        violations_line(&ViolationCounts {
            must,
            should,
            may,
        }),
    ];

    let block = panel_block("HEALTH");
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn violations_line(c: &ViolationCounts) -> Line<'static> {
    Line::from(vec![
        Span::styled("●", Style::default().fg(theme::SEVERITY_MUST)),
        Span::raw(format!(" must {:<3} ", c.must)),
        Span::styled("●", Style::default().fg(theme::SEVERITY_SHOULD)),
        Span::raw(format!(" should {:<3} ", c.should)),
        Span::styled("●", Style::default().fg(theme::SEVERITY_MAY)),
        Span::raw(format!(" may {}", c.may)),
    ])
}

fn render_rules_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let mut lines = vec![
        pair_line("Approved", &d.rules_total.to_string()),
        pair_line("Personal", &d.rules_personal.to_string()),
        Line::from(""),
    ];

    if !d.rules_by_language.is_empty() {
        lines.push(Line::from(Span::styled(
            "By language:",
            theme::header_meta(),
        )));
        for (lang, n) in &d.rules_by_language {
            lines.push(Line::from(format!("  {lang}: {n}")));
        }
    }

    let block = panel_block("RULES");
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn render_drift_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let mut lines = vec![pair_line("Drifted deps", &d.drift_count.to_string())];
    if let Some(last) = &d.last_refresh {
        lines.push(pair_line("Last refresh", last.split('T').next().unwrap_or(last)));
    } else {
        lines.push(pair_line("Last refresh", "never"));
    }
    lines.push(Line::from(""));

    if d.drift_deps.is_empty() && d.drift_count == 0 {
        lines.push(Line::from(Span::styled(
            "No drift — rules are current.",
            Style::default().fg(theme::STATUS_OK),
        )));
    } else {
        for dep in d.drift_deps.iter().take(6) {
            lines.push(Line::from(vec![
                Span::styled("▸ ", Style::default().fg(theme::AMBER)),
                Span::raw(dep.clone()),
            ]));
        }
    }

    let block = panel_block("DRIFT");
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn render_violations_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let mut items: Vec<ListItem> = Vec::new();

    if d.top_violations.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "No violations detected. Nice.",
            Style::default().fg(theme::STATUS_OK),
        ))));
    } else {
        for v in &d.top_violations {
            let sev_color = theme::severity_color(&v.severity);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<7}", v.severity.to_uppercase()),
                    Style::default().fg(sev_color).bold(),
                ),
                Span::styled(
                    format!("{:<30} ", truncate(&v.rule_id, 30)),
                    Style::default().fg(theme::AMBER),
                ),
                Span::raw(format!(
                    "{:<28} ",
                    truncate(&format!("{}:{}", v.file, v.line), 28)
                )),
                Span::styled(
                    truncate(&v.snippet, 40),
                    Style::default().fg(theme::MUTED),
                ),
            ])));
        }
    }

    let total_txt = if d.top_violations.is_empty() {
        "TOP VIOLATIONS".to_string()
    } else {
        format!("TOP VIOLATIONS ({} shown)", d.top_violations.len())
    };
    let block = panel_block(&total_txt);
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_compact(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let d = &app.dashboard;
    let bar_width = (area.width as usize).saturating_sub(20).clamp(6, 24);

    let mut lines = vec![
        gauge_row("Rule system", gauge::render(d.rule_system_score, bar_width)),
        gauge_row("Adherence  ", gauge::render(d.adherence_score, bar_width)),
        Line::from(""),
        pair_line("Rules", &format!("{} approved, {} personal", d.rules_total, d.rules_personal)),
        pair_line("Drift", &format!("{} deps", d.drift_count)),
        Line::from(""),
        Line::from(Span::styled("Top violations", theme::header_meta())),
    ];
    for v in d.top_violations.iter().take(3) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<6}", v.severity.to_uppercase()),
                Style::default().fg(theme::severity_color(&v.severity)).bold(),
            ),
            Span::raw(truncate(&v.rule_id, 22)),
        ]));
    }

    let block = panel_block("DASHBOARD");
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn gauge_row(label: &'static str, gauge_line: Line<'static>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("{label} "), theme::header_meta()),
    ];
    spans.extend(gauge_line.spans);
    Line::from(spans)
}

fn pair_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<14}", label), theme::header_meta()),
        Span::raw(value.to_string()),
    ])
}

fn panel_block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            theme::header_title(),
        ))
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
