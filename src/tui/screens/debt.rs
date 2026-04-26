//! Dedicated debt screen — full list of ranked hotspots with their
//! evidence summary and next-action line. Supports the four data states
//! defined in whetstone-8hm.5.3: not-computed, loading, error, ready.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{
    app::{App, DebtView},
    components::footer,
    theme,
};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("R", "RECOMPUTE"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
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
        DebtView::NotComputed => render_empty(frame, area, "Debt report not computed yet. Press R to compute."),
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
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    let block = block("DEBT");
    frame.render_widget(Paragraph::new(lines).block(block), area);
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
            "  Press R to retry.",
            Style::default().fg(theme::MUTED),
        )),
    ];
    let block = block("DEBT");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_ready(
    frame: &mut Frame<'_>,
    area: Rect,
    summary: &crate::tui::app::DebtSummaryView,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    render_header(frame, rows[0], summary);
    render_hotspots(frame, rows[1], summary);
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    summary: &crate::tui::app::DebtSummaryView,
) {
    let label_color = match summary.debt_label.as_str() {
        "low" => theme::STATUS_OK,
        "moderate" => theme::AMBER,
        _ => theme::STATUS_WARN,
    };
    let lines = vec![Line::from(vec![
        Span::styled("Debt label  ", theme::header_meta()),
        Span::styled(
            summary.debt_label.to_uppercase(),
            Style::default().fg(label_color).bold(),
        ),
        Span::raw(format!("   total {}", summary.finding_count)),
        Span::raw(format!(
            "   dead {}  dup {}  deps {}  hot {}",
            summary.by_dead, summary.by_dup, summary.by_deps, summary.by_hotspots
        )),
    ])];
    frame.render_widget(Paragraph::new(lines).block(block("SUMMARY")), area);
}

fn render_hotspots(
    frame: &mut Frame<'_>,
    area: Rect,
    summary: &crate::tui::app::DebtSummaryView,
) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let (start, end) = window_bounds(summary.selected, summary.hotspots.len(), visible);
    let items: Vec<ListItem> = summary
        .hotspots
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, h)| {
            let title_w = width.saturating_sub(30);
            let prefix = if i == summary.selected { "▶ " } else { "  " };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!("{prefix}{:>2}. ", h.rank),
                        Style::default().fg(theme::MUTED),
                    ),
                    Span::styled(
                        format!("[{}/{}]  ", h.category, h.confidence),
                        Style::default().fg(theme::AMBER),
                    ),
                    Span::styled(
                        format!("score {:.2}  ", h.score),
                        Style::default().fg(theme::MUTED),
                    ),
                    Span::raw(slice_text(&h.title, summary.scroll_x as usize, title_w)),
                ]),
                Line::from(Span::styled(
                    format!("      → {}", slice_text(&h.next_action, summary.scroll_x as usize, width.saturating_sub(8))),
                    Style::default().fg(theme::MUTED),
                )),
            ])
        })
        .collect();

    let total_text = format!(
        "TOP HOTSPOTS ({} shown, {} total findings)",
        summary.hotspots.len(),
        summary.finding_count
    );
    let list = List::new(items).block(block(&total_text));
    frame.render_widget(list, area);
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            theme::header_title(),
        ))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive())
}

fn slice_text(s: &str, start: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if start >= chars.len() {
        return String::new();
    }
    chars[start..chars.len().min(start + width)].iter().collect()
}

fn window_bounds(selected: usize, len: usize, visible: usize) -> (usize, usize) {
    if visible == 0 || len <= visible {
        return (0, len);
    }
    let start = selected.saturating_sub(visible / 2).min(len - visible);
    (start, (start + visible).min(len))
}
