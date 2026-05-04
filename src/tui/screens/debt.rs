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
            if data.detail_selected {
                data.detail_scroll_y = data.detail_scroll_y.saturating_sub(1);
            } else {
                data.selected = data.selected.saturating_sub(1);
                data.detail_scroll_y = 0;
            }
        }
    }

    pub fn select_next(&mut self) {
        if let DebtView::Ready(data) = self {
            if data.detail_selected {
                data.detail_scroll_y = data.detail_scroll_y.saturating_add(1);
            } else {
                let len = data.hotspots.len();
                if len > 0 && data.selected + 1 < len {
                    data.selected += 1;
                    data.detail_scroll_y = 0;
                }
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

pub fn scroll_hint(area: Rect, summary: &DebtSummaryView) -> Option<footer::ScrollHint> {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(8)])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[1]);

    if summary.detail_selected {
        return hint_from_offset(summary.detail_scroll_y, detail_max_scroll(cols[1], summary));
    }

    hint_from_offset(
        summary.selected as u16,
        summary.hotspots.len().saturating_sub(1) as u16,
    )
}

fn render_empty(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DEBT", false)), area);
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
    frame.render_widget(Paragraph::new(lines).block(block("DEBT", false)), area);
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(8)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[1]);

    render_header(frame, rows[0], summary);
    render_issues(frame, cols[0], summary);
    render_detail(frame, cols[1], summary);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let lines = vec![
        Line::from(vec![Span::styled(
            format!("Debt {}", theme::humanize_token(&summary.debt_label)),
            Style::default().fg(theme::AMBER).bold(),
        )]),
        Line::from(vec![
            Span::styled("Total ", Style::default().fg(theme::AMBER).bold()),
            Span::styled(
                summary.finding_count.to_string(),
                Style::default().fg(theme::AMBER).bold(),
            ),
            Span::raw("   "),
            Span::styled("Dead ", theme::header_meta()),
            Span::raw(summary.by_dead.to_string()),
            Span::raw("   "),
            Span::styled("Duplicate ", theme::header_meta()),
            Span::raw(summary.by_dup.to_string()),
            Span::raw("   "),
            Span::styled("Dependency ", theme::header_meta()),
            Span::raw(summary.by_deps.to_string()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("SUMMARY", false)), area);
}

fn render_issues(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = (area.height.saturating_sub(2) / 3).max(1) as usize;
    let (start, end) = window_bounds(summary.selected, summary.hotspots.len(), visible);

    let items: Vec<ListItem> = summary
        .hotspots
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, h)| issue_item(i == summary.selected, h, width))
        .collect();

    let title = format!("ISSUES ({} total findings)", summary.finding_count);
    frame.render_widget(
        List::new(items).block(block(&title, !summary.detail_selected)),
        area,
    );
}

fn issue_item(selected: bool, hotspot: &DebtHotspotRow, width: usize) -> ListItem<'static> {
    let title_w = width.saturating_sub(4).max(16);
    let prefix = if selected { "▶ " } else { "  " };
    let category = display_category(&hotspot.category);
    let category_tag = format!(
        "[{}/{}]",
        category,
        theme::humanize_token(&hotspot.confidence)
    );

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(if selected { theme::AMBER } else { theme::MUTED }),
            ),
            Span::styled(
                truncate(&hotspot.compact_title, title_w),
                Style::default().fg(theme::AMBER).bold(),
            ),
            Span::raw(" "),
            Span::styled(
                format!("({} Impact)", hotspot.impact_level),
                Style::default().fg(ratatui::style::Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                category_tag,
                Style::default().fg(ratatui::style::Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate(&hotspot.primary_file, width.saturating_sub(2)),
                Style::default().fg(theme::MUTED),
            ),
        ]),
    ])
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, summary: &DebtSummaryView) {
    let Some(hotspot) = summary.hotspots.get(summary.selected) else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No hotspot selected.",
                Style::default().fg(theme::MUTED),
            )))
            .block(block("DETAIL", summary.detail_selected)),
            area,
        );
        return;
    };

    let lines = detail_lines(hotspot);
    let effective_scroll = summary
        .detail_scroll_y
        .min(detail_max_scroll(area, summary));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("DETAIL", summary.detail_selected))
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0)),
        area,
    );
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::header_meta()),
        Span::raw(value.to_string()),
    ])
}

fn detail_max_scroll(area: Rect, summary: &DebtSummaryView) -> u16 {
    let Some(hotspot) = summary.hotspots.get(summary.selected) else {
        return 0;
    };

    crate::tui::paragraph_max_scroll(&detail_lines(hotspot), area)
}

fn detail_lines(hotspot: &DebtHotspotRow) -> Vec<Line<'static>> {
    let category = display_category(&hotspot.category);
    let mut lines = vec![
        Line::from(Span::styled(
            hotspot.title.clone(),
            Style::default().fg(theme::AMBER).bold(),
        )),
        Line::from(""),
        detail_line("Rule ID", &hotspot.rule_id),
        detail_line("Category", &category),
        detail_line("Confidence", &theme::humanize_token(&hotspot.confidence)),
        detail_line("Impact", &hotspot.impact_level),
        Line::from(""),
        Line::from(Span::styled("Affected files", theme::header_title())),
    ];
    lines.extend(
        hotspot
            .files
            .iter()
            .map(|file| Line::from(vec![Span::raw("- "), Span::raw(file.clone())])),
    );
    lines.extend([
        Line::from(""),
        Line::from(Span::styled("Snippet", theme::header_title())),
        Line::from(hotspot.snippet.clone()),
    ]);
    lines
}

fn block(title: &str, active: bool) -> Block<'static> {
    Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(if active {
            theme::border_active()
        } else {
            theme::border_inactive()
        })
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

fn hint_from_offset(offset: u16, max_offset: u16) -> Option<footer::ScrollHint> {
    if max_offset == 0 {
        None
    } else {
        Some(footer::ScrollHint {
            up: offset > 0,
            down: offset < max_offset,
        })
    }
}

fn display_category(category: &str) -> String {
    match category {
        "dead" => "Dead".to_string(),
        "dup" => "Duplicate".to_string(),
        "deps" => "Dependency".to_string(),
        other => theme::humanize_token(other),
    }
}

fn window_bounds(selected: usize, len: usize, visible: usize) -> (usize, usize) {
    if visible == 0 || len <= visible {
        return (0, len);
    }
    let start = selected.saturating_sub(visible / 2).min(len - visible);
    (start, (start + visible).min(len))
}
