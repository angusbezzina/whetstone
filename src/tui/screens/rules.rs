//! Rules screen — list + detail of merged approved rules derived from trusted sources.
//!
//! First slice for whetstone-69jb.1: static two-pane layout driven by the
//! four-state `RulesView` enum. The left pane lists merged rule ids with a
//! colored severity badge; the right pane shows full detail (description,
//! layer, language, dep) for the currently selected rule. Keyboard selection is
//! wired via up/down and j/k; the list renders a moving viewport so large rule
//! sets remain navigable.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::tui::{
    app::{App, RulesLanguageFilter},
    components::footer,
    screens::LoadState,
    theme,
};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
}

/// Four data states, shared across list/detail screens.
pub type RulesView = LoadState<RulesData>;

/// One row in the left-hand list. Projected from `LayeredRule` so the
/// renderer doesn't touch the underlying `ApprovedRule` shape.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub id: String,
    pub severity: String,
    pub confidence: String,
    pub languages: Vec<String>,
    /// Derived from the rule id prefix (e.g. `fastapi.async-routes` → `fastapi`).
    /// Falls back to the id itself when there is no `.` separator.
    pub dep: String,
    /// `"project"` or `"personal"` — mirrors `layers::Layer::as_str()`.
    pub layer: String,
    pub source_name: String,
    pub source_url: String,
    pub description: String,
}

/// Everything the renderer needs. Cheap to clone — mostly short strings.
#[derive(Debug, Default, Clone)]
pub struct RulesData {
    pub rows: Vec<RuleRow>,
}

impl RulesView {
    pub fn row_count_for(&self, filter: RulesLanguageFilter) -> usize {
        match self {
            RulesView::Ready(data) => filtered_rows(data, filter).len(),
            _ => 0,
        }
    }
}

/// Synchronously collect the data for this screen. Returns `Error(..)` when
/// the project has no rules at all; callers stay in `NotComputed` until the
/// screen is first opened.
pub fn load(project_dir: &Path) -> RulesView {
    let merged = crate::layers::resolve_merged(project_dir, None, true, true, false);

    if merged.merged.is_empty() {
        // Distinguish "uninitialized project" from "initialized but empty".
        // In both cases we tell the user how to get rules into the system;
        // the message is the same because the fix is the same.
        let initialized = crate::layers::project_is_initialized(project_dir);
        if !initialized {
            let (personal_only, _) = crate::layers::load_personal_only(project_dir, None);
            if personal_only.is_empty() {
                return RulesView::Error("No rules found — run wh init or wh rules add".into());
            }
        }
        // Initialized but no approved rules — same actionable hint.
        return RulesView::Error("No rules found — run wh init or wh rules add".into());
    }

    let rows: Vec<RuleRow> = merged
        .merged
        .iter()
        .map(|lr| {
            let dep = lr
                .rule
                .id
                .split_once('.')
                .map(|(prefix, _)| prefix.to_string())
                .unwrap_or_else(|| lr.rule.id.clone());
            RuleRow {
                id: lr.rule.id.clone(),
                severity: lr.rule.severity.clone(),
                confidence: lr.rule.confidence.clone(),
                languages: vec![lr.rule.language.clone()],
                dep,
                layer: lr.layer.as_str().to_string(),
                source_name: lr.rule.source_name.clone(),
                source_url: lr.rule.source_url.clone(),
                description: lr.rule.description.clone(),
            }
        })
        .collect();
    let rows = group_custom_multi_language_rows(rows);

    RulesView::Ready(Box::new(RulesData { rows }))
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if app.input_mode == crate::tui::app::InputMode::RulesAdd {
        render_add_rule_form(frame, area, app);
        return;
    }

    match &app.dashboard.rules {
        RulesView::NotComputed => render_placeholder(frame, area, "Rules screen not yet loaded."),
        RulesView::Loading => render_placeholder(frame, area, "Loading rules…"),
        RulesView::Error(msg) => render_error(frame, area, msg),
        RulesView::Ready(data) if data.rows.is_empty() => {
            render_placeholder(frame, area, "No approved rules to display.")
        }
        RulesView::Ready(data) => render_ready(frame, area, app, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, app: &App, data: &RulesData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_list(frame, cols[0], app, data);
    render_detail(frame, cols[1], app, data);
}

fn render_list(frame: &mut Frame<'_>, area: Rect, app: &App, data: &RulesData) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new(language_tabs_line(app.rules_ui.language_filter, data))
            .block(block("LANGUAGE", false)),
        panes[0],
    );

    let rows = filtered_rows(data, app.rules_ui.language_filter);
    let selected = app.rules_ui.selected.min(rows.len().saturating_sub(1));

    if rows.is_empty() {
        let lines = vec![Line::from(Span::styled(
            "  No rules in this language.",
            Style::default().fg(theme::MUTED),
        ))];
        frame.render_widget(
            Paragraph::new(lines).block(block("RULE LIST", true)),
            panes[1],
        );
        return;
    }

    let width = panes[1].width.saturating_sub(4) as usize;
    let visible = panes[1].height.saturating_sub(2) as usize;
    let (start, end) = window_bounds(selected, rows.len(), visible);
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, row)| {
            let marker = if i == selected { "▶ " } else { "  " };
            let marker_color = if i == selected {
                theme::AMBER
            } else {
                theme::MUTED
            };
            let id_w = width.saturating_sub(16);
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(marker_color)),
                Span::styled(
                    format!("[{}] ", row.severity.to_uppercase()),
                    Style::default()
                        .fg(theme::severity_color(&row.severity))
                        .bold(),
                ),
                Span::raw(truncate(&row.id, id_w)),
            ]))
        })
        .collect();

    let list = List::new(items).block(block("RULE LIST", true));
    frame.render_widget(list, panes[1]);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, app: &App, data: &RulesData) {
    let rows = filtered_rows(data, app.rules_ui.language_filter);
    let selected = app.rules_ui.selected.min(rows.len().saturating_sub(1));
    let Some(row) = rows.get(selected) else {
        render_placeholder(frame, area, "No rule selected.");
        return;
    };

    let layer_color = if row.layer == "personal" {
        theme::AMBER
    } else {
        theme::MUTED
    };

    let lines = vec![
        kv_line(area.width, "ID", &row.id, Style::default().bold()),
        kv_line(
            area.width,
            "Severity",
            &row.severity.to_uppercase(),
            Style::default()
                .fg(theme::severity_color(&row.severity))
                .bold(),
        ),
        kv_line(area.width, "Confidence", &row.confidence, Style::default()),
        kv_line(
            area.width,
            "Language",
            &display_languages(&row.languages),
            Style::default(),
        ),
        kv_line(area.width, "Source", &source_label(row), Style::default()),
        kv_line(
            area.width,
            "Layer",
            &row.layer,
            Style::default().fg(layer_color).bold(),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Description",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from(row.description.clone()),
    ];

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block("DETAIL", false));
    frame.render_widget(paragraph, area);
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("RULES", true)), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    if msg.contains("No rules found") {
        return render_placeholder(frame, area, "No rules yet.");
    }
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Rules compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("RULES", true)), area);
}

fn render_add_rule_form(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let form = &app.rules_ui.form;
    let lang = match form.language_idx {
        0 => "TS",
        1 => "Rust",
        2 => "Python",
        _ => "All",
    };
    let severity = crate::tui::app::rule_form_severity(form.severity_idx).to_uppercase();
    let lines = vec![
        Line::from(Span::styled(
            "Tab next field · ←/→ change scope/language/severity · Enter submit · Esc cancel",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        form_line(
            "Scope",
            if form.team_scope { "Team" } else { "Personal" },
            form.active_field == 0,
        ),
        form_line("Name", &form.name, form.active_field == 1),
        form_line("Language", lang, form.active_field == 2),
        form_line("Severity", &severity, form.active_field == 3),
        Line::from(""),
        Line::from(Span::styled("Rule Text", theme::header_meta())),
        Line::from(Span::styled(
            if form.rule_text.is_empty() {
                "—"
            } else {
                &form.rule_text
            },
            if form.active_field == 4 {
                Style::default().fg(theme::AMBER).bold()
            } else {
                Style::default().fg(ratatui::style::Color::White)
            },
        )),
        Line::from(""),
        Line::from(Span::styled(
            form.error.clone().unwrap_or_default(),
            Style::default().fg(theme::STATUS_WARN),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("ADD RULE", false))
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
        Span::styled(format!("{label:<12}"), theme::header_meta()),
        Span::styled(value_text.to_string(), value_style),
    ])
}

fn block(title: &str, show_add_cta: bool) -> Block<'static> {
    let mut block = Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());
    if show_add_cta {
        block = block.title_top(
            Line::from(Span::styled(
                " Press A to Add a Rule ",
                Style::default().fg(theme::AMBER).bold(),
            ))
            .right_aligned(),
        );
    }
    block
}

fn group_custom_multi_language_rows(rows: Vec<RuleRow>) -> Vec<RuleRow> {
    let mut grouped: std::collections::BTreeMap<String, Vec<RuleRow>> =
        std::collections::BTreeMap::new();
    let mut passthrough = Vec::new();

    for row in rows {
        if let Some(base_id) = custom_multi_language_base_id(&row.id) {
            grouped.entry(base_id).or_default().push(row);
        } else {
            passthrough.push(row);
        }
    }

    let mut combined = passthrough;
    for (base_id, mut group) in grouped {
        if group.len() == 1 {
            combined.push(group.pop().expect("single-item group must contain a row"));
            continue;
        }

        let first = group.remove(0);
        let mut languages = first.languages.clone();
        for row in group {
            languages.extend(row.languages);
        }
        sort_languages(&mut languages);
        languages.dedup();

        combined.push(RuleRow {
            id: base_id,
            severity: first.severity,
            confidence: first.confidence,
            languages,
            dep: first.dep,
            layer: first.layer,
            source_name: first.source_name,
            source_url: first.source_url,
            description: first.description,
        });
    }

    combined.sort_by(|a, b| a.id.cmp(&b.id));
    combined
}

fn custom_multi_language_base_id(id: &str) -> Option<String> {
    for language in ["python", "rust", "typescript"] {
        let suffix = format!("-{language}");
        if id.starts_with("custom.") && id.ends_with(&suffix) {
            return Some(id.trim_end_matches(&suffix).to_string());
        }
    }
    None
}

fn filtered_rows(data: &RulesData, filter: RulesLanguageFilter) -> Vec<&RuleRow> {
    data.rows
        .iter()
        .filter(|row| row_matches_filter(row, filter))
        .collect()
}

fn row_matches_filter(row: &RuleRow, filter: RulesLanguageFilter) -> bool {
    match filter {
        RulesLanguageFilter::All => true,
        RulesLanguageFilter::Python => row.languages.iter().any(|lang| lang == "python"),
        RulesLanguageFilter::Rust => row.languages.iter().any(|lang| lang == "rust"),
        RulesLanguageFilter::Typescript => row.languages.iter().any(|lang| lang == "typescript"),
    }
}

fn language_tabs_line(active: RulesLanguageFilter, data: &RulesData) -> Line<'static> {
    let tab = |label: String, is_active: bool| {
        if is_active {
            Span::styled(
                format!("[{label}]"),
                Style::default().fg(theme::AMBER).bold(),
            )
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(theme::MUTED))
        }
    };

    let all = filtered_rows(data, RulesLanguageFilter::All).len();
    let python = filtered_rows(data, RulesLanguageFilter::Python).len();
    let rust = filtered_rows(data, RulesLanguageFilter::Rust).len();
    let typescript = filtered_rows(data, RulesLanguageFilter::Typescript).len();

    Line::from(vec![
        tab(format!("All {all}"), active == RulesLanguageFilter::All),
        Span::raw(" "),
        tab(
            format!("Python {python}"),
            active == RulesLanguageFilter::Python,
        ),
        Span::raw(" "),
        tab(format!("Rust {rust}"), active == RulesLanguageFilter::Rust),
        Span::raw(" "),
        tab(
            format!("TS {typescript}"),
            active == RulesLanguageFilter::Typescript,
        ),
    ])
}

fn display_languages(languages: &[String]) -> String {
    if languages.len() == 3
        && languages.iter().any(|lang| lang == "python")
        && languages.iter().any(|lang| lang == "rust")
        && languages.iter().any(|lang| lang == "typescript")
    {
        return "all".to_string();
    }

    languages.join(", ")
}

fn sort_languages(languages: &mut [String]) {
    languages.sort_by_key(|language| match language.as_str() {
        "python" => 0,
        "rust" => 1,
        "typescript" => 2,
        _ => 3,
    });
}

fn source_label(row: &RuleRow) -> String {
    let is_authored_rule =
        row.id.starts_with("custom.") || row.source_url.starts_with("personal://");
    if is_authored_rule {
        return if row.layer == "personal" {
            "Personal Rule".to_string()
        } else {
            "Team Rule".to_string()
        };
    }

    if row.source_name == row.dep {
        "Dependency".to_string()
    } else {
        "Source".to_string()
    }
}

fn kv_line(width: u16, label: &str, value: &str, value_style: Style) -> Line<'static> {
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
        Span::styled(value_text, value_style),
    ])
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

    fn mk_row(id: &str, severity: &str, layer: &str) -> RuleRow {
        RuleRow {
            id: id.to_string(),
            severity: severity.to_string(),
            confidence: "high".to_string(),
            languages: vec!["python".to_string()],
            dep: id
                .split_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default(),
            layer: layer.to_string(),
            source_name: id
                .split_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default(),
            source_url: format!("https://example.com/{id}"),
            description: format!("A sample rule called {id}."),
        }
    }

    fn buffer_string(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect()
    }

    /// Byte-safe prefix for assertion error messages. Truncates on a char
    /// boundary so box-drawing glyphs (multi-byte) don't panic the slice.
    fn preview(s: &str, max: usize) -> String {
        s.chars().take(max).collect()
    }

    #[test]
    fn render_shows_ready_rule_ids() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_ready_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();

        // Short ids so both fit in the narrow left list pane at 80x24.
        let rows = vec![
            mk_row("fa.async", "must", "project"),
            mk_row("pd.strict", "should", "personal"),
        ];
        app.dashboard.rules = RulesView::Ready(Box::new(RulesData { rows }));

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered = buffer_string(&terminal);
        assert!(
            rendered.contains("fa.async"),
            "expected first rule id in buffer; got: {}",
            preview(&rendered, 400)
        );
        assert!(
            rendered.contains("pd.strict"),
            "expected second rule id in buffer; got: {}",
            preview(&rendered, 400)
        );
        assert!(
            rendered.contains("Press A to Add a Rule"),
            "expected add-rule CTA in buffer; got: {}",
            preview(&rendered, 400)
        );
        assert!(
            rendered.contains("LANGUAGE"),
            "expected language tabs block in buffer; got: {}",
            preview(&rendered, 400)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn render_shows_error_message() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_error_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();

        app.dashboard.rules = RulesView::Error("boom".into());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered = buffer_string(&terminal);
        assert!(
            rendered.contains("boom"),
            "expected error message in buffer; got: {}",
            preview(&rendered, 400)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn render_detail_shows_source_label_and_value() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_detail_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.dashboard.rules = RulesView::Ready(Box::new(RulesData {
            rows: vec![mk_row("fa.async", "must", "project")],
        }));

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered = buffer_string(&terminal);
        assert!(
            rendered.contains("Source"),
            "detail pane should show source field; got: {}",
            preview(&rendered, 400)
        );
        assert!(
            rendered.contains("Dependency"),
            "detail pane should classify dependency-backed rules; got: {}",
            preview(&rendered, 400)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn source_label_maps_authored_rules_by_scope() {
        let mut row = mk_row("custom.personal-rule", "should", "personal");
        row.source_name = "custom".to_string();
        row.source_url = "personal://custom/custom.personal-rule".to_string();
        assert_eq!(source_label(&row), "Personal Rule");

        row.layer = "project".to_string();
        assert_eq!(source_label(&row), "Team Rule");
    }

    #[test]
    fn source_label_maps_non_dependency_rules_to_source() {
        let mut row = mk_row("handbook.write-tests", "should", "project");
        row.source_name = "engineering-handbook".to_string();
        assert_eq!(source_label(&row), "Source");
    }

    #[test]
    fn render_add_rule_form_shows_severity_field() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_form_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.input_mode = crate::tui::app::InputMode::RulesAdd;

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered = buffer_string(&terminal);
        assert!(
            rendered.contains("Severity"),
            "add-rule form should show severity field; got: {}",
            preview(&rendered, 400)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_groups_custom_all_language_rules_into_one_row() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_group_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        for language in ["python", "rust", "typescript"] {
            crate::rule_authoring::add(
                &tmp,
                crate::rule_authoring::AddOptions {
                    rule_id: &format!("custom.grouped-{language}"),
                    description: "Grouped rule",
                    match_regex: None,
                    severity: "should",
                    confidence: "high",
                    category: "convention",
                    language,
                    source_url: None,
                    dep: Some("custom"),
                    personal: true,
                },
            )
            .unwrap();
        }

        let view = load(&tmp);
        let RulesView::Ready(data) = view else {
            panic!("expected rules view to load successfully");
        };

        assert_eq!(data.rows.len(), 1, "expected grouped all-language rule");
        assert_eq!(data.rows[0].id, "custom.grouped");
        assert_eq!(display_languages(&data.rows[0].languages), "all");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
