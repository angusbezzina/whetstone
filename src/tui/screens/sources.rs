//! Unified Sources screen — combines internal dependency/doc sources with
//! handpicked trusted personal/team sources and lets users shape project taste.

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
    screens::LoadState,
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
}

pub type SourcesView = LoadState<SourcesData>;

#[derive(Debug, Default, Clone)]
pub struct SourcesData {
    pub dependencies: Vec<crate::tui::screens::extract::WorklistRow>,
    pub dependency_error: Option<String>,
    pub project: Vec<SourceRow>,
    pub personal: Vec<SourceRow>,
}

#[derive(Debug, Default, Clone)]
pub struct SourceRow {
    pub name: String,
    pub lang: Option<String>,
    pub kind: Option<String>,
    pub last_fetched: Option<String>,
    pub fetch_state: Option<String>,
    pub source_confidence: Option<String>,
    pub confidence_guidance: Option<String>,
}

#[derive(Debug, Clone)]
struct SourceListRow {
    name: String,
    lang: String,
    source_type: String,
}

pub fn load(project_dir: &Path) -> SourcesView {
    match crate::source_mgmt::list(project_dir) {
        Ok(value) => {
            let (dependencies, dependency_error) =
                match crate::tui::screens::extract::load_data(project_dir) {
                    Ok(data) => (data.entries, None),
                    Err(e) => (Vec::new(), Some(e.to_string())),
                };
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
                dependencies,
                dependency_error,
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
            (Self::Ready(data), SourcesDataset::Dependencies) => data.dependencies.len(),
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
    let fetch_state = entry
        .get("fetch_state")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source_confidence = entry
        .get("source_confidence")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let confidence_guidance = entry
        .get("confidence_guidance")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    SourceRow {
        name,
        lang,
        kind,
        last_fetched,
        fetch_state,
        source_confidence,
        confidence_guidance,
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    render_sources_list(frame, cols[0], app);
    if app.input_mode == InputMode::SourcesAdd {
        render_add_form(frame, cols[1], app);
    } else {
        render_selected_detail(frame, cols[1], app);
    }
}

pub fn scroll_hint(area: Rect, app: &App) -> Option<footer::ScrollHint> {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    if app.sources_ui.detail_selected {
        let max_scroll = match app.sources_ui.dataset {
            SourcesDataset::Dependencies => dependency_detail_max_scroll(cols[1], app),
            SourcesDataset::Personal => custom_detail_max_scroll(cols[1], app, true),
            SourcesDataset::Team => custom_detail_max_scroll(cols[1], app, false),
        };
        return hint_from_offset(app.sources_ui.detail_scroll_y, max_scroll);
    }

    let (selected, max_selected) = match app.sources_ui.dataset {
        SourcesDataset::Dependencies => (
            app.sources_ui.selected as u16,
            app.dashboard
                .sources
                .row_count_for(SourcesDataset::Dependencies)
                .saturating_sub(1) as u16,
        ),
        SourcesDataset::Personal => (
            app.sources_ui.selected as u16,
            app.dashboard
                .sources
                .row_count_for(SourcesDataset::Personal)
                .saturating_sub(1) as u16,
        ),
        SourcesDataset::Team => (
            app.sources_ui.selected as u16,
            app.dashboard
                .sources
                .row_count_for(SourcesDataset::Team)
                .saturating_sub(1) as u16,
        ),
    };

    hint_from_offset(selected, max_selected)
}

fn render_internal_source_detail(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_placeholder(
            frame,
            area,
            "Dependency source->rule worklist is not loaded yet.",
        ),
        SourcesView::Loading => render_placeholder(frame, area, "Loading source->rule worklist…"),
        SourcesView::Error(msg) => render_error(frame, area, msg),
        SourcesView::Ready(data) => {
            if let Some(msg) = &data.dependency_error {
                render_error(frame, area, msg);
            } else if data.dependencies.is_empty() {
                render_placeholder(
                    frame,
                    area,
                    "No source->rule worklist entries are available. Run wh init to generate them.",
                );
            } else {
                let extract_data = crate::tui::screens::extract::ExtractData {
                    entries: data.dependencies.clone(),
                    selected: app.sources_ui.selected,
                };
                crate::tui::screens::extract::render_detail_scrolled(
                    frame,
                    area,
                    &extract_data,
                    app.sources_ui.detail_scroll_y,
                    app.sources_ui.detail_selected,
                );
            }
        }
    }
}

fn render_sources_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = match app.sources_ui.dataset {
        SourcesDataset::Dependencies => dependency_rows(app),
        SourcesDataset::Personal => personal_rows(app),
        SourcesDataset::Team => team_rows(app),
    };
    let selected = app.sources_ui.selected;
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    frame.render_widget(
        Paragraph::new(dataset_tabs_line(app.sources_ui.dataset))
            .block(block("DATASET", !app.sources_ui.detail_selected)),
        panes[0],
    );

    let block = block("SOURCE LIST", !app.sources_ui.detail_selected);
    if rows.is_empty() {
        let lines = vec![Line::from(Span::styled(
            "  No sources in this dataset.",
            Style::default().fg(theme::MUTED),
        ))];
        frame.render_widget(Paragraph::new(lines).block(block), panes[1]);
        return;
    }

    let visible = panes[1].height.saturating_sub(2) as usize;
    let start = selected
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(visible));
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, row)| ListItem::new(source_row_line(i == selected, row)))
        .collect();

    frame.render_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(theme::AMBER)),
        panes[1],
    );
}

fn dataset_tabs_line(active: SourcesDataset) -> Line<'static> {
    let tab = |label: &str, is_active: bool| {
        if is_active {
            Span::styled(
                format!("[{label}]"),
                Style::default().fg(theme::AMBER).bold(),
            )
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
    let form = &app.sources_ui.form;
    let scope = if form.team_scope { "Team" } else { "Personal" };
    let language = source_form_language_label(form.language_idx);
    let lines = vec![
        Line::from(Span::styled(
            format!("Adding to the {scope} source list."),
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            "Tab next field · ←/→ change language · Enter submit · Esc cancel",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        form_line("URL", &form.url, form.active_field == 0),
        form_line("Language", language, form.active_field == 1),
        Line::from(""),
        Line::from(Span::styled(
            form.error.clone().unwrap_or_default(),
            Style::default().fg(theme::STATUS_WARN),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("DETAIL", true))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_selected_detail(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.sources_ui.dataset {
        SourcesDataset::Dependencies => render_internal_source_detail(frame, area, app),
        SourcesDataset::Personal => render_custom_detail(frame, area, app, true),
        SourcesDataset::Team => render_custom_detail(frame, area, app, false),
    }
}

fn render_custom_detail(frame: &mut Frame<'_>, area: Rect, app: &App, personal: bool) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_custom_detail_message(
            frame,
            area,
            "Handpicked sources are not loaded yet.",
            app.sources_ui.detail_selected,
        ),
        SourcesView::Loading => render_custom_detail_message(
            frame,
            area,
            "Loading handpicked sources…",
            app.sources_ui.detail_selected,
        ),
        SourcesView::Error(msg) => {
            render_custom_detail_message(frame, area, msg, app.sources_ui.detail_selected)
        }
        SourcesView::Ready(data) => {
            let rows = if personal {
                &data.personal
            } else {
                &data.project
            };
            let Some(row) = rows.get(app.sources_ui.selected) else {
                render_custom_detail_message(
                    frame,
                    area,
                    "No source selected.",
                    app.sources_ui.detail_selected,
                );
                return;
            };
            let mut lines = vec![kv_line(area.width, "Name", &row.name)];
            if let Some(lang) = row.lang.as_deref() {
                lines.push(kv_line(area.width, "Language", lang));
            }
            if let Some(kind) = row.kind.as_deref() {
                lines.push(kv_line(area.width, "Type", kind));
            }
            if let Some(last) = &row.last_fetched {
                lines.push(kv_line(area.width, "Last fetched", last));
            }
            if let Some(fetch_state) = row.fetch_state.as_deref() {
                lines.push(kv_line(area.width, "Fetch health", fetch_state));
            }
            if let Some(conf) = row.source_confidence.as_deref() {
                lines.push(kv_line(area.width, "Source confidence", conf));
            }
            if let Some(guidance) = row.confidence_guidance.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    truncate(guidance, area.width.saturating_sub(4) as usize),
                    Style::default().fg(theme::MUTED),
                )));
            }
            let effective_scroll = app
                .sources_ui
                .detail_scroll_y
                .min(crate::tui::paragraph_max_scroll(&lines, area));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(detail_block(app.sources_ui.detail_selected))
                    .wrap(Wrap { trim: false })
                    .scroll((effective_scroll, 0)),
                area,
            );
        }
    }
}

fn dependency_rows(app: &App) -> Vec<SourceListRow> {
    match &app.dashboard.sources {
        SourcesView::Ready(data) => data
            .dependencies
            .iter()
            .map(|row| SourceListRow {
                name: row.name.clone(),
                lang: language_short(Some(&row.language)).to_string(),
                source_type: source_kind_badge(row.source_type.as_deref()),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn personal_rows(app: &App) -> Vec<SourceListRow> {
    match &app.dashboard.sources {
        SourcesView::Ready(data) => data.personal.iter().map(format_source_row).collect(),
        _ => Vec::new(),
    }
}

fn team_rows(app: &App) -> Vec<SourceListRow> {
    match &app.dashboard.sources {
        SourcesView::Ready(data) => data.project.iter().map(format_source_row).collect(),
        _ => Vec::new(),
    }
}

fn format_source_row(row: &SourceRow) -> SourceListRow {
    SourceListRow {
        name: row.name.clone(),
        lang: language_short(row.lang.as_deref()).to_string(),
        source_type: source_kind_badge(row.kind.as_deref()),
    }
}

fn source_row_line(selected: bool, row: &SourceListRow) -> Line<'static> {
    let prefix = if selected { "> " } else { "  " };
    let mut spans = vec![Span::styled(prefix, Style::default().fg(theme::MUTED))];
    spans.push(Span::styled(
        row.name.clone(),
        Style::default().fg(theme::AMBER).bold(),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("({})", row.lang),
        Style::default().fg(ratatui::style::Color::White),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("[{}]", row.source_type),
        Style::default().fg(theme::MUTED),
    ));
    Line::from(spans)
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
    match kind.unwrap_or("custom") {
        "llms_full_txt" => "LLMsFullTxt".to_string(),
        "llms_txt" => "LLMsTxt".to_string(),
        "readme" => "Readme".to_string(),
        "html_converted" => "HtmlConverted".to_string(),
        other => theme::humanize_token(other).replace(' ', ""),
    }
}

fn kv_line(width: u16, label: &str, value: &str) -> Line<'static> {
    let label_text = label.to_string();
    let available = width.saturating_sub(2) as usize;
    let max_value_width = available.saturating_sub(label_text.chars().count() + 1);
    let value_text = truncate(value, max_value_width.max(1));
    let spacer_len = available
        .saturating_sub(label_text.chars().count())
        .saturating_sub(value_text.chars().count())
        .max(1);
    let spacer = " ".repeat(spacer_len);
    Line::from(vec![
        Span::styled(label_text, theme::header_meta()),
        Span::raw(spacer),
        Span::styled(
            value_text,
            Style::default().fg(ratatui::style::Color::White),
        ),
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
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES", false)), area);
}

fn render_custom_detail_message(frame: &mut Frame<'_>, area: Rect, message: &str, active: bool) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(detail_block(active)), area);
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
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES", false)), area);
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

fn truncate(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let kept: String = value.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

fn custom_detail_max_scroll(area: Rect, app: &App, personal: bool) -> u16 {
    let SourcesView::Ready(data) = &app.dashboard.sources else {
        return 0;
    };
    let rows = if personal {
        &data.personal
    } else {
        &data.project
    };
    let Some(row) = rows.get(app.sources_ui.selected) else {
        return 0;
    };

    let mut lines = vec![kv_line(area.width, "Name", &row.name)];
    if let Some(lang) = row.lang.as_deref() {
        lines.push(kv_line(area.width, "Language", lang));
    }
    if let Some(kind) = row.kind.as_deref() {
        lines.push(kv_line(area.width, "Type", kind));
    }
    if let Some(last) = &row.last_fetched {
        lines.push(kv_line(area.width, "Last fetched", last));
    }
    if let Some(fetch_state) = row.fetch_state.as_deref() {
        lines.push(kv_line(area.width, "Fetch health", fetch_state));
    }
    if let Some(conf) = row.source_confidence.as_deref() {
        lines.push(kv_line(area.width, "Source confidence", conf));
    }
    if let Some(guidance) = row.confidence_guidance.as_deref() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            truncate(guidance, area.width.saturating_sub(4) as usize),
            Style::default().fg(theme::MUTED),
        )));
    }
    crate::tui::paragraph_max_scroll(&lines, area)
}

fn dependency_detail_max_scroll(area: Rect, app: &App) -> u16 {
    match &app.dashboard.sources {
        SourcesView::Ready(data)
            if data.dependency_error.is_none() && !data.dependencies.is_empty() =>
        {
            let extract_data = crate::tui::screens::extract::ExtractData {
                entries: data.dependencies.clone(),
                selected: app.sources_ui.selected,
            };
            crate::tui::screens::extract::detail_max_scroll(area, &extract_data)
        }
        _ => 0,
    }
}

fn detail_block(active: bool) -> Block<'static> {
    block("DETAIL", active).title_top(
        Line::from(Span::styled(
            " Press A to add a source ",
            Style::default().fg(theme::AMBER).bold(),
        ))
        .right_aligned(),
    )
}

fn source_form_language_label(idx: usize) -> &'static str {
    match idx {
        0 => "TS",
        1 => "Python",
        2 => "Rust",
        _ => "All",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{app::App, screens::extract::WorklistRow};
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
        app.dashboard.sources = SourcesView::Ready(Box::new(SourcesData {
            dependencies: vec![WorklistRow {
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
                source_confidence: Some("high".into()),
                confidence_guidance: Some(
                    "High-confidence source; normal extraction flow is safe.".into(),
                ),
                fetch_health: Some("fresh".into()),
                source_age_days: Some(1),
                reason: None,
                sections: vec![],
            }],
            dependency_error: None,
            project: vec![SourceRow {
                name: "team-style".into(),
                lang: None,
                kind: Some("team_guide".into()),
                last_fetched: None,
                fetch_state: Some("fresh".into()),
                source_confidence: Some("high".into()),
                confidence_guidance: Some("High-confidence source; normal extraction flow is safe.".into()),
            }],
            personal: vec![SourceRow {
                name: "my-notes".into(),
                lang: None,
                kind: None,
                last_fetched: None,
                fetch_state: Some("stale".into()),
                source_confidence: Some("low".into()),
                confidence_guidance: Some("Low-confidence source; limit candidates to clearly documented, directly cited rules.".into()),
            }],
        }));

        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        let out = rendered(&terminal);
        assert!(out.contains("SOURCE LIST"));
        assert!(out.contains("Dependencies"));
        assert!(out.contains("pydantic"));
        assert_eq!(language_short(Some("python")), "PY");
        assert_eq!(source_kind_badge(Some("llms_full_txt")), "LLMsFullTxt");
    }
}
