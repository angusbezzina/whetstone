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
    app::{App, InputMode, SourcesDataset},
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
            }))
        }
        Err(e) => SourcesView::Error(e.to_string()),
    }
}

impl SourcesView {
    pub fn row_count_for(&self, dataset: SourcesDataset) -> usize {
        match (self, dataset) {
            (Self::Ready(data), SourcesDataset::Personal) => data.personal.len(),
            (Self::Ready(data), SourcesDataset::Team) => data.project.len(),
            _ => 0,
        }
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
    if app.input_mode == InputMode::SourcesAdd {
        render_add_form(frame, area, app);
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    render_sources_list(frame, cols[0], app);
    render_selected_detail(frame, cols[1], app);
}

fn render_internal_source_detail(frame: &mut Frame<'_>, area: Rect, app: &App) {
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
            crate::tui::screens::extract::render_detail(frame, area, data);
        }
    }
}

fn render_sources_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = match app.sources_dataset {
        SourcesDataset::Dependencies => dependency_rows(app),
        SourcesDataset::Personal => personal_rows(app),
        SourcesDataset::Team => team_rows(app),
    };
    let selected = match app.sources_dataset {
        SourcesDataset::Dependencies => match &app.dashboard.extract {
            crate::tui::screens::extract::ExtractView::Ready(data) => data.selected,
            _ => 0,
        },
        SourcesDataset::Personal | SourcesDataset::Team => app.sources_selected,
    };
    let block = block("SOURCE LIST");
    if rows.is_empty() {
        let lines = vec![
            dataset_tabs_line(app.sources_dataset),
            Line::from(""),
            Line::from(Span::styled(
                "  No sources in this dataset.",
                Style::default().fg(theme::MUTED),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    let visible = area.height.saturating_sub(3) as usize;
    let start = selected
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(visible));
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, row)| {
            let mut line = row.clone();
            if i == selected {
                line = format!("> {line}");
            } else {
                line = format!("  {line}");
            }
            let style = if i == selected {
                Style::default().fg(theme::AMBER).bold()
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();

    frame.render_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(theme::AMBER)),
        area,
    );
    let tab_area = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1);
    frame.render_widget(Paragraph::new(dataset_tabs_line(app.sources_dataset)), tab_area);
}

fn dataset_tabs_line(active: SourcesDataset) -> Line<'static> {
    let tab = |label: &str, is_active: bool| {
        if is_active {
            Span::styled(format!("[{label}]"), Style::default().fg(theme::AMBER).bold())
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(theme::MUTED))
        }
    };
    Line::from(vec![
        tab("Dependencies", active == SourcesDataset::Dependencies),
        Span::raw(" "),
        tab("Personal", active == SourcesDataset::Personal),
        Span::raw(" "),
        tab("Team", active == SourcesDataset::Team),
    ])
}

fn render_add_form(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let form = &app.sources_form;
    let mut lines = vec![
        Line::from(Span::styled(
            "Press A to add a handpicked source.",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            "Tab next field · T toggle Personal/Team · Enter save · Esc cancel",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
    ];

    let editing = true;
    lines.push(form_line(
        "Scope",
        if form.team_scope { "Team" } else { "Personal" },
        false,
    ));
    lines.push(form_line("URL/Path", &form.url, editing && form.active_field == 0));
    lines.push(form_line("Name", &form.name, editing && form.active_field == 1));
    if let Some(err) = &form.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(err.clone(), Style::default().fg(theme::STATUS_WARN))));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("ADD SOURCE"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_selected_detail(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.sources_dataset {
        SourcesDataset::Dependencies => render_internal_source_detail(frame, area, app),
        SourcesDataset::Personal => render_custom_detail(frame, area, app, true),
        SourcesDataset::Team => render_custom_detail(frame, area, app, false),
    }
}

fn render_custom_detail(frame: &mut Frame<'_>, area: Rect, app: &App, personal: bool) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_placeholder(frame, area, "Handpicked sources are not loaded yet."),
        SourcesView::Loading => render_placeholder(frame, area, "Loading handpicked sources…"),
        SourcesView::Error(msg) => render_error(frame, area, msg),
        SourcesView::Ready(data) => {
            let rows = if personal { &data.personal } else { &data.project };
            let Some(row) = rows.get(app.sources_selected) else {
                render_placeholder(frame, area, "No source selected.");
                return;
            };
            let mut lines = vec![
                kv_line("Name", &row.name),
                kv_line("Language", row.lang.as_deref().unwrap_or("Any")),
                kv_line("Type", row.kind.as_deref().unwrap_or("Custom")),
            ];
            if let Some(last) = &row.last_fetched {
                lines.push(kv_line("Last fetched", last));
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .block(block("DETAIL"))
                    .wrap(Wrap { trim: false }),
                area,
            );
        }
    }
}

fn dependency_rows(app: &App) -> Vec<String> {
    match &app.dashboard.extract {
        crate::tui::screens::extract::ExtractView::Ready(data) => data
            .entries
            .iter()
            .map(|row| {
                format!(
                    "{} ({}) [{}]",
                    row.name,
                    language_short(Some(&row.language)),
                    source_kind_badge(row.source_type.as_deref())
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn personal_rows(app: &App) -> Vec<String> {
    match &app.dashboard.sources {
        SourcesView::Ready(data) => data.personal.iter().map(format_source_row).collect(),
        _ => Vec::new(),
    }
}

fn team_rows(app: &App) -> Vec<String> {
    match &app.dashboard.sources {
        SourcesView::Ready(data) => data.project.iter().map(format_source_row).collect(),
        _ => Vec::new(),
    }
}

fn format_source_row(row: &SourceRow) -> String {
    format!(
        "{} ({}) [{}]",
        row.name,
        language_short(row.lang.as_deref()),
        source_kind_badge(row.kind.as_deref())
    )
}

fn language_short(lang: Option<&str>) -> &'static str {
    match lang.unwrap_or("any").to_ascii_lowercase().as_str() {
        "typescript" | "ts" => "TS",
        "python" | "py" => "PY",
        "rust" | "rs" => "RS",
        _ => "ANY",
    }
}

fn source_kind_badge(kind: Option<&str>) -> String {
    kind.map(theme::humanize_token)
        .unwrap_or_else(|| "Custom".to_string())
        .replace(' ', "")
}

fn kv_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), theme::header_meta()),
        Span::styled(value.to_string(), Style::default().fg(ratatui::style::Color::White)),
    ])
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
    fn render_shows_dependency_list_in_row_format() {
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
        }));

        terminal.draw(|frame| render(frame, frame.area(), &app)).unwrap();
        let out = rendered(&terminal);
        assert!(out.contains("SOURCE LIST"));
        assert!(out.contains("Dependencies"));
        assert!(out.contains("pydantic"));
        assert_eq!(language_short(Some("python")), "PY");
        assert_eq!(source_kind_badge(Some("llms_full_txt")), "LlmsFullTxt");
    }
}
