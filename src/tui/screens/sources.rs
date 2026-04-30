//! Unified Sources screen — combines internal dependency/doc sources with
//! handpicked personal/team sources and lets users add a custom source.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, InputMode},
    components::footer,
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
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
    pub scroll: usize,
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
            SourcesView::Ready(Box::new(SourcesData {
                project,
                personal,
                scroll: 0,
            }))
        }
        Err(e) => SourcesView::Error(e.to_string()),
    }
}

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
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(14), Constraint::Length(12)])
        .split(area);

    render_internal_sources(frame, rows[0], app);
    render_handpicked_and_form(frame, rows[1], app);
}

fn render_internal_sources(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.extract {
        crate::tui::screens::extract::ExtractView::NotComputed => render_placeholder(
            frame,
            area,
            "Internal sources are not loaded yet.",
        ),
        crate::tui::screens::extract::ExtractView::Loading => {
            render_placeholder(frame, area, "Loading internal sources…")
        }
        crate::tui::screens::extract::ExtractView::Error(msg) => render_error(frame, area, msg),
        crate::tui::screens::extract::ExtractView::Ready(data) if data.entries.is_empty() => {
            render_placeholder(
                frame,
                area,
                "No internal sources are available right now. Run wh init to generate them.",
            )
        }
        crate::tui::screens::extract::ExtractView::Ready(data) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
                .split(area);
            crate::tui::screens::extract::render_worklist(frame, cols[0], data);
            crate::tui::screens::extract::render_detail(frame, cols[1], data);
        }
    }
}

fn render_handpicked_and_form(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    render_handpicked_sources(frame, cols[0], app);
    render_add_form(frame, cols[1], app);
}

fn render_handpicked_sources(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_placeholder(frame, area, "Handpicked sources are not loaded yet."),
        SourcesView::Loading => render_placeholder(frame, area, "Loading handpicked sources…"),
        SourcesView::Error(msg) => render_error(frame, area, msg),
        SourcesView::Ready(data) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            render_column(frame, cols[0], "PERSONAL", &data.personal, data.scroll);
            render_column(frame, cols[1], "TEAM", &data.project, data.scroll);
        }
    }
}

fn render_column(frame: &mut Frame<'_>, area: Rect, title: &str, rows: &[SourceRow], scroll: usize) {
    let block = block(title);
    if rows.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No handpicked sources in this scope.",
                Style::default().fg(theme::MUTED),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    let visible = area.height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = rows
        .iter()
        .skip(scroll)
        .take(visible)
        .map(row_to_item)
        .collect();
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

fn render_add_form(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let form = &app.sources_form;
    let mut lines = vec![
        Line::from(Span::styled(
            "Press A to add a handpicked source.",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            "While editing: Tab next field · T toggle Personal/Team · Enter save · Esc cancel",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
    ];

    let editing = app.input_mode == InputMode::SourcesAdd;
    lines.push(form_line("Scope", if form.team_scope { "Team" } else { "Personal" }, editing && form.active_field == usize::MAX));
    lines.push(form_line("URL", &form.url, editing && form.active_field == 0));
    lines.push(form_line("Name", &form.name, editing && form.active_field == 1));
    lines.push(form_line("Language", &form.language, editing && form.active_field == 2));
    lines.push(form_line("Kind", &form.kind, editing && form.active_field == 3));
    if let Some(err) = &form.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(err.clone(), Style::default().fg(theme::STATUS_WARN))));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("ADD HANDPICKED SOURCE"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn form_line(label: &str, value: &str, active: bool) -> Line<'static> {
    let value_text = if value.is_empty() { "—" } else { value };
    let value_style = if active {
        Style::default().fg(theme::AMBER).bold()
    } else {
        Style::default().fg(ratatui::style::Color::White)
    };
    Line::from(vec![
        Span::styled(format!("{label:<10}"), theme::header_meta()),
        Span::styled(value_text.to_string(), value_style),
    ])
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
            "  Sources failed to load:",
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
    use crate::tui::{app::App, screens::extract::{ExtractData, ExtractView, WorklistRow}};
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
    fn render_shows_internal_and_handpicked_sources() {
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app();
        app.dashboard.extract = ExtractView::Ready(Box::new(ExtractData {
            entries: vec![WorklistRow {
                name: "pydantic".into(),
                language: "python".into(),
                priority: "ready_now".into(),
                utility_percent: 89,
                next_step: "do it".into(),
                remaining_quota: 5,
                source_type: Some("llms_full_txt".into()),
                source_url: Some("https://docs.pydantic.dev".into()),
                version: Some(">=2.10.0".into()),
                registry: Some("pypi".into()),
                freshness_confidence: Some("high".into()),
                source_age_days: Some(1),
                reason: None,
                sections: vec![],
            }],
            selected: 0,
            total: 1,
        }));
        app.dashboard.sources = SourcesView::Ready(Box::new(SourcesData {
            project: vec![SourceRow { name: "team-style".into(), lang: None, kind: Some("team_guide".into()), last_fetched: None }],
            personal: vec![SourceRow { name: "my-notes".into(), lang: None, kind: None, last_fetched: None }],
            scroll: 0,
        }));

        terminal.draw(|frame| render(frame, frame.area(), &app)).unwrap();
        let out = rendered(&terminal);
        assert!(out.contains("CORE PACKAGES"));
        assert!(out.contains("HANDPICKED") || out.contains("ADD HANDPICKED SOURCE"));
        assert!(out.contains("pydantic"));
        assert!(out.contains("team-style"));
        assert!(out.contains("my-notes"));
    }
}
