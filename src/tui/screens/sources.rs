//! Sources screen — subscription manager for committed + personal layers.
//!
//! Renders the two-layer list of custom sources tracked by
//! [`crate::source_mgmt::list`] — one column per layer (PROJECT / PERSONAL).
//! Supports the four view states defined in whetstone-8hm.5.3:
//! NotComputed, Loading, Error, Ready.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("R", "REFRESH"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
}

#[derive(Default, Clone)]
pub enum SourcesView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<SourcesData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct SourcesData {
    pub project: Vec<SourceRow>,
    pub personal: Vec<SourceRow>,
}

#[derive(Debug, Default, Clone)]
pub struct SourceRow {
    pub name: String,
    pub lang: Option<String>,
    pub kind: Option<String>,
    pub last_fetched: Option<String>,
}

pub fn load(project_dir: &Path) -> SourcesView {
    match crate::source_mgmt::list(project_dir) {
        Ok(value) => {
            let project = value
                .get("project")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().map(row_from_json).collect())
                .unwrap_or_default();
            let personal = value
                .get("personal")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().map(row_from_json).collect())
                .unwrap_or_default();
            SourcesView::Ready(Box::new(SourcesData { project, personal }))
        }
        Err(e) => SourcesView::Error(e.to_string()),
    }
}

/// Project a single JSON entry from `source_mgmt::list` into a [`SourceRow`].
///
/// Field names mirror `CustomSource`: `url`, `name`, `language`, `source_kind`.
/// `last_fetched` is not yet emitted by `list` — kept as `None` so the
/// renderer is ready when source-fetch telemetry lands.
fn row_from_json(entry: &serde_json::Value) -> SourceRow {
    let url_fallback = entry
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let name = entry
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| url_fallback.to_string());
    let lang = entry
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let kind = entry
        .get("source_kind")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let last_fetched = entry
        .get("last_fetched")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    SourceRow {
        name,
        lang,
        kind,
        last_fetched,
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_placeholder(
            frame,
            area,
            "Sources screen not yet loaded. Press R to compute.",
        ),
        SourcesView::Loading => render_placeholder(frame, area, "Loading sources…"),
        SourcesView::Error(msg) => render_error(frame, area, msg),
        SourcesView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &SourcesData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_column(frame, cols[0], "PROJECT", &data.project);
    render_column(frame, cols[1], "PERSONAL", &data.personal);
}

fn render_column(frame: &mut Frame<'_>, area: Rect, title: &str, rows: &[SourceRow]) {
    let block = block(title);
    if rows.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No subscriptions in this layer.",
                Style::default().fg(theme::MUTED),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    let items: Vec<ListItem> = rows.iter().map(row_to_item).collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn row_to_item(row: &SourceRow) -> ListItem<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        row.name.clone(),
        Style::default().fg(theme::AMBER),
    ));

    let badge = match (row.lang.as_deref(), row.kind.as_deref()) {
        (Some(l), Some(k)) => Some(format!(" ({l}/{k})")),
        (Some(l), None) => Some(format!(" ({l})")),
        (None, Some(k)) => Some(format!(" ({k})")),
        (None, None) => None,
    };
    if let Some(b) = badge {
        spans.push(Span::styled(b, Style::default().fg(theme::MUTED)));
    }

    if let Some(ts) = &row.last_fetched {
        spans.push(Span::styled(
            format!(" — {ts}"),
            Style::default().fg(theme::MUTED),
        ));
    }

    ListItem::new(Line::from(spans))
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Sources compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES")), area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn make_app() -> App {
        let tmp = std::env::temp_dir().join(format!(
            "wh_tui_sources_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&tmp);
        App::new(&tmp).expect("app should construct against a tmp project dir")
    }

    fn rendered(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect()
    }

    #[test]
    fn render_shows_source_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app();
        app.dashboard.sources = SourcesView::Ready(Box::new(SourcesData {
            project: vec![SourceRow {
                name: "fastapi-docs".to_string(),
                lang: Some("python".to_string()),
                kind: Some("llms-txt".to_string()),
                last_fetched: None,
            }],
            personal: vec![SourceRow {
                name: "my-notes".to_string(),
                lang: None,
                kind: None,
                last_fetched: None,
            }],
        }));

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let out = rendered(&terminal);
        assert!(
            out.contains("fastapi-docs"),
            "expected project source name to render; got: {}",
            &out[..out.len().min(400)]
        );
        assert!(
            out.contains("my-notes"),
            "expected personal source name to render; got: {}",
            &out[..out.len().min(400)]
        );
        assert!(out.contains("PROJECT"));
        assert!(out.contains("PERSONAL"));
    }

    #[test]
    fn render_shows_error_message() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app();
        app.dashboard.sources = SourcesView::Error("boom".into());

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let out = rendered(&terminal);
        assert!(
            out.contains("boom"),
            "expected error message to render; got: {}",
            &out[..out.len().min(400)]
        );
        assert!(out.contains("compute failed"));
    }

    #[test]
    fn render_ready_with_empty_layers_shows_muted_hint() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app();
        app.dashboard.sources = SourcesView::Ready(Box::default());

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let out = rendered(&terminal);
        assert!(
            out.contains("No subscriptions in this layer."),
            "expected empty-layer hint; got: {}",
            &out[..out.len().min(400)]
        );
    }
}
