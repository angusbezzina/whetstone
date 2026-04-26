//! Dedicated extract screen — per-dependency extraction worklist with a
//! detail pane for the currently selected entry. Supports the four data
//! states defined in whetstone-8hm.5.3: not-computed, loading, error, ready.
//!
//! Data source: `whetstone/.state/extraction-handoff.json`, loaded via
//! `crate::worklist::load`. If that artifact is missing (no `wh init` yet)
//! we surface an Error with a helpful next-step.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

use crate::tui::{
    app::App,
    components::footer,
    theme,
};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("R", "REFRESH"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
}

/// Four-state view for the Extract screen; mirrors DebtView on the debt screen.
#[derive(Default, Clone)]
pub enum ExtractView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<ExtractData>),
    Error(String),
}

/// Fully-hydrated Extract screen data. Populated by [`load`] on first open.
#[derive(Debug, Clone, Default)]
pub struct ExtractData {
    pub entries: Vec<WorklistRow>,
    pub selected: usize,
    pub total: u32,
}

impl ExtractView {
    pub fn select_prev(&mut self) {
        if let ExtractView::Ready(data) = self {
            data.selected = data.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if let ExtractView::Ready(data) = self {
            let len = data.entries.len();
            if len > 0 && data.selected + 1 < len {
                data.selected += 1;
            }
        }
    }
}

/// One row in the worklist — the fields we surface in the TUI. Derived
/// from `crate::worklist::load`'s per-entry JSON object.
#[derive(Debug, Clone)]
pub struct WorklistRow {
    pub name: String,
    pub language: String,
    pub priority: String,
    pub score: f64,
    pub next_step: String,
    pub existing_rules: u32,
}

/// Read the on-disk worklist and project it into `ExtractData`. Returns
/// `ExtractView::Error(..)` when the handoff artifact is missing or
/// unreadable so the screen can explain the next step.
pub fn load(project_dir: &Path) -> ExtractView {
    match crate::worklist::load(project_dir) {
        Err(e) => ExtractView::Error(format!(
            "{e}\nRun `wh init` (or `wh reinit`) to generate a worklist."
        )),
        Ok(value) => {
            let entries = project_entries(&value);
            let total = entries.len() as u32;
            ExtractView::Ready(Box::new(ExtractData {
                entries,
                selected: 0,
                total,
            }))
        }
    }
}

fn project_entries(handoff: &Value) -> Vec<WorklistRow> {
    let arr = handoff
        .get("worklist")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.iter()
        .map(|e| WorklistRow {
            name: e
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            language: e
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            priority: e
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            score: e.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
            next_step: e
                .get("next_step")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            existing_rules: e
                .get("existing_rules")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        })
        .collect()
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.extract {
        ExtractView::NotComputed => render_empty(
            frame,
            area,
            "Extraction worklist not loaded yet. Press R to load.",
        ),
        ExtractView::Loading => render_empty(frame, area, "Loading worklist…"),
        ExtractView::Error(msg) => render_error(frame, area, msg),
        ExtractView::Ready(data) if data.entries.is_empty() => render_empty(
            frame,
            area,
            "Worklist is empty — run wh init to generate one.",
        ),
        ExtractView::Ready(data) => render_ready(frame, area, data),
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
    let block = block("EXTRACT");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Worklist load failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
    ];
    for part in msg.lines() {
        lines.push(Line::from(format!("  {part}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press R to retry.",
        Style::default().fg(theme::MUTED),
    )));

    let block = block("EXTRACT");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_worklist(frame, cols[0], data);
    render_detail(frame, cols[1], data);
}

fn render_worklist(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let (start, end) = window_bounds(data.selected, data.entries.len(), visible);
    let items: Vec<ListItem> = data
        .entries
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, row)| {
            let rank = i + 1;
            let is_selected = i == data.selected;
            let priority_color = priority_color(&row.priority);
            let name_style = if is_selected {
                Style::default().fg(theme::AMBER).bold()
            } else {
                Style::default()
            };

            let lang_label = if row.language.is_empty() {
                "—".to_string()
            } else {
                row.language.clone()
            };

            let title_line = Line::from(vec![
                Span::styled(
                    format!("{rank:>3}.  "),
                    Style::default().fg(theme::MUTED),
                ),
                Span::styled(truncate(&row.name, width.saturating_sub(12)), name_style),
            ]);
            let meta_line = Line::from(vec![
                Span::styled(
                    format!("{lang_label:<10}"),
                    Style::default().fg(theme::MUTED),
                ),
                Span::raw(" · "),
                Span::styled(
                    row.priority.clone(),
                    Style::default().fg(priority_color),
                ),
                Span::raw(" · "),
                Span::styled(
                    format!("score {:.1}", row.score),
                    Style::default().fg(theme::MUTED),
                ),
            ]);

            ListItem::new(vec![title_line, meta_line])
        })
        .collect();

    let title = format!("WORKLIST ({} total)", data.total);
    let list = List::new(items).block(block(&title));
    frame.render_widget(list, area);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
    let Some(row) = data.entries.get(data.selected) else {
        let block = block("DETAIL");
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No entry selected.",
                Style::default().fg(theme::MUTED),
            )))
            .block(block),
            area,
        );
        return;
    };

    let priority_color = priority_color(&row.priority);
    let language = if row.language.is_empty() {
        "—".to_string()
    } else {
        row.language.clone()
    };

    let lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Name      ", Style::default().fg(theme::MUTED)),
            Span::styled(row.name.clone(), Style::default().fg(theme::AMBER).bold()),
        ]),
        Line::from(vec![
            Span::styled("Language  ", Style::default().fg(theme::MUTED)),
            Span::raw(language),
        ]),
        Line::from(vec![
            Span::styled("Priority  ", Style::default().fg(theme::MUTED)),
            Span::styled(row.priority.clone(), Style::default().fg(priority_color)),
        ]),
        Line::from(vec![
            Span::styled("Score     ", Style::default().fg(theme::MUTED)),
            Span::raw(format!("{:.2}", row.score)),
        ]),
        Line::from(vec![
            Span::styled("Existing  ", Style::default().fg(theme::MUTED)),
            Span::raw(format!("{} rule(s)", row.existing_rules)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "What the score means",
            theme::header_title(),
        )),
        Line::from(Span::styled(
            "Higher scores mean this dependency is a better next extraction target based on docs/source quality and freshness.",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Next step",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(row.next_step.clone()),
    ];

    let block = block("DETAIL");
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

fn priority_color(priority: &str) -> ratatui::style::Color {
    match priority {
        "ready_now" => theme::STATUS_OK,
        "resolved_low" => theme::AMBER,
        "pending" => theme::MUTED,
        "failed" => theme::STATUS_WARN,
        _ => theme::MUTED,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};

    fn synthetic_app() -> App {
        let tmp =
            std::env::temp_dir().join(format!("wh_tui_extract_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        App::new(&tmp).expect("App::new")
    }

    #[test]
    fn render_shows_worklist_entries() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = synthetic_app();
        app.dashboard.extract = ExtractView::Ready(Box::new(ExtractData {
            entries: vec![
                WorklistRow {
                    name: "fastapi".into(),
                    language: "python".into(),
                    priority: "ready_now".into(),
                    score: 125.0,
                    next_step: "Read the linked source, draft up to 3 rule(s)".into(),
                    existing_rules: 2,
                },
                WorklistRow {
                    name: "react".into(),
                    language: "typescript".into(),
                    priority: "resolved_low".into(),
                    score: 55.0,
                    next_step: "Source is llms_txt — proceed carefully".into(),
                    existing_rules: 0,
                },
            ],
            selected: 0,
            total: 2,
        }));

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect();

        assert!(
            rendered.contains("fastapi"),
            "expected fastapi in worklist; got: {}",
            &rendered[..rendered.len().min(600)]
        );
        assert!(
            rendered.contains("react"),
            "expected react in worklist; got: {}",
            &rendered[..rendered.len().min(600)]
        );
    }

    #[test]
    fn render_shows_error_message() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = synthetic_app();
        app.dashboard.extract = ExtractView::Error("boom".into());

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect();

        assert!(
            rendered.contains("boom"),
            "expected error message to contain 'boom'; got: {}",
            &rendered[..rendered.len().min(600)]
        );
    }

    #[test]
    fn load_missing_handoff_returns_error() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_tui_extract_missing_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let view = load(&tmp);
        match view {
            ExtractView::Error(msg) => {
                assert!(
                    msg.contains("wh init") || msg.contains("wh reinit"),
                    "expected next-step hint referencing wh init/reinit, got: {msg}"
                );
            }
            _ => panic!("expected ExtractView::Error when handoff is missing"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
