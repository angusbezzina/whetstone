//! Dedicated debt screen — full list of ranked hotspots with a richer detail
//! pane for the selected finding.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, DebtHotspotRow, DebtSummaryView, DebtView},
    components::footer,
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
}

impl DebtView {
    pub fn select_prev(&mut self) {
        if let DebtView::Ready(data) = self {
            data.selected = data.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if let DebtView::Ready(data) = self {
            let len = data.hotspots.len();
            if len > 0 && data.selected + 1 < len {
                data.selected += 1;
            }
        }
    }

    pub fn scroll_left(&mut self, cols: u16) {
        if let DebtView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_sub(cols);
        }
    }

    pub fn scroll_right(&mut self, cols: u16) {
        if let DebtView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_add(cols);
        }
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.debt {
        DebtView::NotComputed => render_empty(frame, area, "Debt report not computed yet."),
        DebtView::Loading => render_empty(frame, area, "Computing debt…"),
        DebtView::Error(msg) => render_error(frame, area, msg),
        DebtView::Ready(summary) if summary.hotspots.is_empty() => render_empty(
            frame,
            area,
            "No hotspots at the current confidence threshold. Debt looks clean.",
        ),
        DebtView::Ready(summary) => render_ready(frame, area, summary),
    }
}

fn render_empty(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {message}"), Style::default().fg(theme::MUTED))),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DEBT")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Debt compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
        Line::from(""),
        Line::from(Span::styled(
            "  Exit and reopen the TUI to retry.",
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DEBT")), area);
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[1]);

    render_header(frame, rows[0], summary);
    render_hotspots(frame, cols[0], summary);
    render_detail(frame, cols[1], summary);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let lines = vec![Line::from(vec![
        Span::styled("Debt  ", theme::header_meta()),
        Span::styled(
            theme::humanize_token(&summary.debt_label),
            Style::default()
                .fg(theme::debt_label_color(&summary.debt_label))
                .bold(),
        ),
        Span::raw("   "),
        Span::styled("Total  ", theme::header_meta()),
        Span::raw(summary.finding_count.to_string()),
        Span::raw("   "),
        Span::styled("Dead  ", theme::header_meta()),
        Span::raw(summary.by_dead.to_string()),
        Span::raw("   "),
        Span::styled("Duplicate  ", theme::header_meta()),
        Span::raw(summary.by_dup.to_string()),
        Span::raw("   "),
        Span::styled("Dependency  ", theme::header_meta()),
        Span::raw(summary.by_deps.to_string()),
        Span::raw("   "),
        Span::styled("Hotspot  ", theme::header_meta()),
        Span::raw(summary.by_hotspots.to_string()),
    ])];
    frame.render_widget(Paragraph::new(lines).block(block("SUMMARY")), area);
}

fn render_hotspots(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let (start, end) = window_bounds(summary.selected, summary.hotspots.len(), visible);

    let items: Vec<ListItem> = summary
        .hotspots
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, h)| hotspot_item(i == summary.selected, h, width))
        .collect();

    let title = format!("TOP HOTSPOTS ({} total findings)", summary.finding_count);
    frame.render_widget(List::new(items).block(block(&title)), area);
}

fn hotspot_item(selected: bool, hotspot: &DebtHotspotRow, width: usize) -> ListItem<'static> {
    let title_w = width.saturating_sub(30).max(16);
    let prefix = if selected { "▶ " } else { "  " };

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(if selected { theme::AMBER } else { theme::MUTED }),
            ),
            Span::styled(
                format!(
                    "[{}/{}]  ",
                    theme::humanize_token(&hotspot.category),
                    theme::humanize_token(&hotspot.confidence)
                ),
                Style::default().fg(theme::AMBER),
            ),
            Span::styled(
                format!("{} (Impact: {}%)", truncate(&hotspot.compact_title, title_w), hotspot.impact_percent),
                Style::default().fg(theme::debt_label_color("moderate")).bold(),
            ),
        ]),
        Line::from(Span::styled(
            format!("      {}", truncate(&hotspot.primary_file, width.saturating_sub(6))),
            Style::default().fg(theme::MUTED),
        )),
    ])
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let Some(hotspot) = summary.hotspots.get(summary.selected) else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No hotspot selected.",
                Style::default().fg(theme::MUTED),
            )))
            .block(block("DETAIL")),
            area,
        );
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(hotspot.title.clone(), Style::default().fg(theme::AMBER).bold())),
        Line::from(""),
        detail_line("Hotspot ID", &hotspot.id),
        detail_line("Rule ID", &hotspot.rule_id),
        detail_line("Category", &theme::humanize_token(&hotspot.category)),
        detail_line("Confidence", &theme::humanize_token(&hotspot.confidence)),
        detail_line(
            "Impact Score",
            &format!("{}% (relative to the highest-impact finding in this report)", hotspot.impact_percent),
        ),
        detail_line("File Count", &hotspot.files.len().to_string()),
        Line::from(""),
        Line::from(Span::styled("Why this was flagged", theme::header_title())),
    ];

    for item in &hotspot.evidence_summary {
        lines.push(Line::from(Span::styled(
            format!("• {}", item),
            Style::default().fg(theme::MUTED),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Affected files", theme::header_title())));
    for file in hotspot.files.iter().take(8) {
        lines.push(Line::from(format!("• {}", file)));
    }
    if hotspot.files.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("• … +{} more", hotspot.files.len() - 8),
            Style::default().fg(theme::MUTED),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("DETAIL"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::header_meta()),
        Span::raw(value.to_string()),
    ])
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive())
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn window_bounds(selected: usize, len: usize, visible: usize) -> (usize, usize) {
    if visible == 0 || len <= visible {
        return (0, len);
    }
    let start = selected.saturating_sub(visible / 2).min(len - visible);
    (start, (start + visible).min(len))
}
