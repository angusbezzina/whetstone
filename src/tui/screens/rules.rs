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
    pub member_ids: Vec<String>,
    pub severity: String,
    pub confidence: String,
    pub category: String,
    pub languages: Vec<String>,
    /// Derived from the rule id prefix (e.g. `fastapi.async-routes` → `fastapi`).
    /// Falls back to the id itself when there is no `.` separator.
    pub dep: String,
    /// `"project"` or `"personal"` — mirrors `layers::Layer::as_str()`.
    pub layer: String,
    pub source_name: String,
    pub source_url: String,
    pub description: String,
    pub match_patterns: Vec<String>,
    pub lint_bindings: Vec<crate::rules::ApprovedLintBinding>,
    pub formatter: Option<crate::rules::ApprovedFormatterDirective>,
    pub tests: Vec<crate::rules::ApprovedTestBinding>,
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

    pub fn selected_row(&self, filter: RulesLanguageFilter, selected: usize) -> Option<RuleRow> {
        match self {
            RulesView::Ready(data) => filtered_rows(data, filter)
                .get(selected)
                .map(|row| (*row).clone()),
            _ => None,
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
                member_ids: vec![lr.rule.id.clone()],
                severity: lr.rule.severity.clone(),
                confidence: lr.rule.confidence.clone(),
                category: lr.rule.category.clone(),
                languages: lr.rule.languages.clone(),
                dep,
                layer: lr.layer.as_str().to_string(),
                source_name: lr.rule.source_name.clone(),
                source_url: lr.rule.source_url.clone(),
                description: lr.rule.description.clone(),
                match_patterns: lr
                    .rule
                    .signals
                    .iter()
                    .filter_map(|signal| signal.match_pattern.clone())
                    .collect(),
                lint_bindings: lr
                    .rule
                    .signals
                    .iter()
                    .flat_map(crate::rules::approved_signal_lint_bindings)
                    .collect(),
                formatter: lr.rule.formatter.clone(),
                tests: lr.rule.tests.clone(),
            }
        })
        .collect();
    let rows = group_custom_multi_language_rows(rows);

    RulesView::Ready(Box::new(RulesData { rows }))
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if matches!(
        app.input_mode,
        crate::tui::app::InputMode::RulesAdd | crate::tui::app::InputMode::RulesEdit
    ) {
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

pub fn scroll_hint(area: Rect, app: &App) -> Option<footer::ScrollHint> {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    if app.rules_ui.detail_selected {
        let max_offset = detail_max_scroll(cols[1], app)?;
        return hint_from_offset(app.rules_ui.detail_scroll_y, max_offset);
    }

    hint_from_offset(
        app.rules_ui.selected as u16,
        app.dashboard
            .rules
            .row_count_for(app.rules_ui.language_filter)
            .saturating_sub(1) as u16,
    )
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, app: &App, data: &RulesData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
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
        Paragraph::new(language_tabs_line(app.rules_ui.language_filter, data)).block(block(
            "LANGUAGE",
            !app.rules_ui.detail_selected,
            false,
        )),
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
            Paragraph::new(lines).block(block("RULE SET", !app.rules_ui.detail_selected, true)),
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

    let list = List::new(items).block(block("RULE SET", !app.rules_ui.detail_selected, true));
    frame.render_widget(list, panes[1]);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, app: &App, data: &RulesData) {
    let rows = filtered_rows(data, app.rules_ui.language_filter);
    let selected = app.rules_ui.selected.min(rows.len().saturating_sub(1));
    let Some(row) = rows.get(selected) else {
        render_placeholder(frame, area, "No rule selected.");
        return;
    };

    let mut lines = vec![
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
        kv_line(area.width, "Category", &row.category, Style::default()),
        kv_line(
            area.width,
            "Language",
            &display_languages(&row.languages),
            Style::default(),
        ),
        kv_line(area.width, "Source", &source_label(row), Style::default()),
        kv_line(
            area.width,
            "Enforcement",
            &enforcement_summary(row),
            Style::default(),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Description",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from(row.description.clone()),
    ];

    let enforcement_lines = enforcement_detail_lines(row);
    if !enforcement_lines.is_empty() {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Enforcement Details",
                Style::default().fg(theme::MUTED),
            )),
            Line::from(""),
        ]);
        lines.extend(enforcement_lines);
    }

    if let Some(message) = &app.rules_ui.detail_message {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                message.clone(),
                Style::default().fg(theme::STATUS_WARN),
            )),
        ]);
    }

    let effective_scroll = app
        .rules_ui
        .detail_scroll_y
        .min(crate::tui::paragraph_max_scroll(&lines, area));

    let paragraph = Paragraph::new(lines)
        .scroll((effective_scroll, 0))
        .wrap(Wrap { trim: false })
        .block(detail_block(row, app.rules_ui.detail_selected));
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
    frame.render_widget(
        Paragraph::new(lines).block(block("RULES", true, true)),
        area,
    );
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
    frame.render_widget(
        Paragraph::new(lines).block(block("RULES", true, true)),
        area,
    );
}

fn render_add_rule_form(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let form = &app.rules_ui.form;
    let is_edit = app.input_mode == crate::tui::app::InputMode::RulesEdit;
    let lang = match form.language_idx {
        0 => "TS",
        1 => "Rust",
        2 => "Python",
        _ => "All",
    };
    let severity = crate::tui::app::rule_form_severity(form.severity_idx).to_uppercase();
    let mode = crate::tui::app::rule_form_mode_label(form.mode_idx);
    let mut lines = vec![
        form_line(
            "Scope",
            if form.team_scope { "Team" } else { "Personal" },
            form.active_field == 0,
        ),
        form_line("Name", &form.name, form.active_field == 1),
        form_line("Language", lang, form.active_field == 2),
        form_line("Severity", &severity, form.active_field == 3),
        form_line("Mode", mode, form.active_field == 4),
        Line::from(""),
        Line::from(Span::styled("Rule Text", theme::header_meta())),
        Line::from(Span::styled(
            if form.rule_text.is_empty() {
                "—"
            } else {
                &form.rule_text
            },
            if form.active_field == 5 {
                Style::default().fg(theme::AMBER).bold()
            } else {
                Style::default().fg(ratatui::style::Color::White)
            },
        )),
    ];

    let mode_rows = mode_specific_form_rows(form);
    if !mode_rows.is_empty() {
        for (field_idx, label, value) in mode_rows {
            lines.push(Line::from(""));
            lines.push(form_line(label, &value, form.active_field == field_idx));
        }
    }

    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            form.error.clone().unwrap_or_default(),
            Style::default().fg(theme::STATUS_WARN),
        )),
    ]);

    frame.render_widget(
        Paragraph::new(lines)
            .block(form_block(
                if is_edit { "EDIT RULE" } else { "ADD RULE" },
                "Tab next field · ←/→ wrap scope/language/severity/mode · Enter submit · Esc cancel",
            ))
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

fn block(title: &str, active: bool, show_add_cta: bool) -> Block<'static> {
    let mut block = Block::default()
        .title(Span::styled(format!(" {title} "), theme::header_title()))
        .borders(Borders::ALL)
        .border_style(if active {
            theme::border_active()
        } else {
            theme::border_inactive()
        });
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

fn form_block(title: &str, instructions: &str) -> Block<'static> {
    block(title, true, false).title_bottom(
        Line::from(Span::styled(
            format!(" {instructions} "),
            Style::default().fg(theme::AMBER).bold(),
        ))
        .centered(),
    )
}

fn detail_block(row: &RuleRow, active: bool) -> Block<'static> {
    let title = if is_authored_rule(row) {
        " E Edit · D Delete "
    } else {
        " Read-only rule "
    };
    block("DETAIL", active, false).title_top(
        Line::from(Span::styled(
            title,
            Style::default().fg(theme::AMBER).bold(),
        ))
        .right_aligned(),
    )
}

fn group_custom_multi_language_rows(rows: Vec<RuleRow>) -> Vec<RuleRow> {
    let mut grouped: std::collections::BTreeMap<String, Vec<RuleRow>> =
        std::collections::BTreeMap::new();

    for row in rows {
        grouped.entry(group_row_key(&row)).or_default().push(row);
    }

    let mut combined = Vec::new();
    for (group_id, mut group) in grouped {
        if group.len() == 1 {
            let mut row = group.pop().expect("single-item group must contain a row");
            row.id = group_id;
            combined.push(row);
            continue;
        }

        let first = group.remove(0);
        let mut member_ids: Vec<String> = first
            .member_ids
            .iter()
            .cloned()
            .chain(group.iter().flat_map(|row| row.member_ids.iter().cloned()))
            .collect();
        member_ids.sort();
        member_ids.dedup();
        let mut languages = first.languages.clone();
        for row in group {
            languages.extend(row.languages);
        }
        sort_languages(&mut languages);
        languages.dedup();

        combined.push(RuleRow {
            id: group_id,
            member_ids,
            severity: first.severity,
            confidence: first.confidence,
            category: first.category,
            languages,
            dep: first.dep,
            layer: first.layer,
            source_name: first.source_name,
            source_url: first.source_url,
            description: first.description,
            match_patterns: first.match_patterns,
            lint_bindings: first.lint_bindings,
            formatter: first.formatter,
            tests: first.tests,
        });
    }

    combined.sort_by(|a, b| a.id.cmp(&b.id));
    combined
}

fn group_row_key(row: &RuleRow) -> String {
    custom_multi_language_base_id(&row.id).unwrap_or_else(|| row.id.clone())
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
        RulesLanguageFilter::All => is_all_language_row(row),
        RulesLanguageFilter::Python => row.languages.as_slice() == ["python"],
        RulesLanguageFilter::Rust => row.languages.as_slice() == ["rust"],
        RulesLanguageFilter::Typescript => row.languages.as_slice() == ["typescript"],
    }
}

fn language_tabs_line(active: RulesLanguageFilter, data: &RulesData) -> Line<'static> {
    let tab = |name: &str, count: usize, is_active: bool| -> Vec<Span<'static>> {
        let label_style = if is_active {
            Style::default().fg(theme::AMBER).bold()
        } else {
            Style::default().fg(theme::MUTED)
        };
        let count_style = if is_active {
            Style::default().fg(theme::AMBER).bold().italic()
        } else {
            Style::default().fg(theme::MUTED).italic()
        };
        vec![
            Span::styled(if is_active { "[" } else { " " }, label_style),
            Span::styled(name.to_string(), label_style),
            Span::raw(" "),
            Span::styled(format!("({count})"), count_style),
            Span::styled(if is_active { "]" } else { " " }, label_style),
        ]
    };

    let all = filtered_rows(data, RulesLanguageFilter::All).len();
    let python = filtered_rows(data, RulesLanguageFilter::Python).len();
    let rust = filtered_rows(data, RulesLanguageFilter::Rust).len();
    let typescript = filtered_rows(data, RulesLanguageFilter::Typescript).len();

    let mut spans = Vec::new();
    spans.extend(tab("All", all, active == RulesLanguageFilter::All));
    spans.push(Span::raw(" "));
    spans.extend(tab("Python", python, active == RulesLanguageFilter::Python));
    spans.push(Span::raw(" "));
    spans.extend(tab("Rust", rust, active == RulesLanguageFilter::Rust));
    spans.push(Span::raw(" "));
    spans.extend(tab(
        "TS",
        typescript,
        active == RulesLanguageFilter::Typescript,
    ));
    Line::from(spans)
}

fn display_languages(languages: &[String]) -> String {
    if is_all_languages(languages) {
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

fn is_all_language_row(row: &RuleRow) -> bool {
    is_all_languages(&row.languages)
}

fn is_all_languages(languages: &[String]) -> bool {
    languages.len() == 3
        && languages.iter().any(|lang| lang == "python")
        && languages.iter().any(|lang| lang == "rust")
        && languages.iter().any(|lang| lang == "typescript")
}

fn source_label(row: &RuleRow) -> String {
    if is_authored_rule(row) {
        return if row.layer == "personal" {
            "Personal".to_string()
        } else {
            "Team".to_string()
        };
    }

    if !row.source_name.is_empty() && row.source_name != row.dep {
        row.source_name.clone()
    } else {
        format!("{} (Dependency)", row.dep)
    }
}

fn is_authored_rule(row: &RuleRow) -> bool {
    row.id.starts_with("custom.") || row.source_url.starts_with("personal://")
}

fn enforcement_summary(row: &RuleRow) -> String {
    if !row.match_patterns.is_empty() {
        return format!("pattern ({})", row.match_patterns[0]);
    }
    if let Some(binding) = row.lint_bindings.first() {
        return format!("linter ({} {})", binding.tool, binding.code);
    }
    if let Some(formatter) = &row.formatter {
        return format!("formatter ({})", formatter.tool);
    }
    if let Some(test) = row.tests.first() {
        return format!("test ({})", test.runner);
    }
    "advisory".to_string()
}

fn enforcement_detail_lines(row: &RuleRow) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for pattern in &row.match_patterns {
        lines.push(Line::from(format!("Pattern: {pattern}")));
    }
    for lint in &row.lint_bindings {
        lines.push(Line::from(format!("Lint: {} {}", lint.tool, lint.code)));
    }
    if let Some(formatter) = &row.formatter {
        lines.push(Line::from(format!("Formatter: {}", formatter.tool)));
        for (key, value) in &formatter.options {
            lines.push(Line::from(format!("  {key} = {value}")));
        }
    }
    for test in &row.tests {
        let selector = test
            .selector
            .as_ref()
            .map(|selector| format!(" :: {selector}"))
            .unwrap_or_default();
        lines.push(Line::from(format!(
            "Test: {} {}{}",
            test.runner, test.path, selector
        )));
    }
    lines
}

fn mode_specific_form_rows(
    form: &crate::tui::app::RulesFormState,
) -> Vec<(usize, &'static str, String)> {
    match form.mode_idx {
        1 => vec![(6, "Regex", form.detail_a.clone())],
        2 => vec![
            (6, "Lint Tool", form.detail_a.clone()),
            (7, "Lint Code", form.detail_b.clone()),
        ],
        3 => vec![
            (6, "Fmt Tool", form.detail_a.clone()),
            (7, "Fmt Key", form.detail_b.clone()),
            (8, "Fmt Value", form.detail_c.clone()),
        ],
        4 => vec![
            (6, "Runner", form.detail_a.clone()),
            (7, "Test Path", form.detail_b.clone()),
            (8, "Selector", form.detail_c.clone()),
        ],
        _ => Vec::new(),
    }
}

fn detail_max_scroll(area: Rect, app: &App) -> Option<u16> {
    let crate::tui::screens::LoadState::Ready(data) = &app.dashboard.rules else {
        return None;
    };
    let rows = filtered_rows(data, app.rules_ui.language_filter);
    let selected = app.rules_ui.selected.min(rows.len().saturating_sub(1));
    let row = rows.get(selected)?;

    let mut lines = vec![
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
        kv_line(area.width, "Category", &row.category, Style::default()),
        kv_line(
            area.width,
            "Language",
            &display_languages(&row.languages),
            Style::default(),
        ),
        kv_line(area.width, "Source", &source_label(row), Style::default()),
        kv_line(
            area.width,
            "Enforcement",
            &enforcement_summary(row),
            Style::default(),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Description",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from(row.description.clone()),
    ];
    let enforcement_lines = enforcement_detail_lines(row);
    if !enforcement_lines.is_empty() {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Enforcement Details",
                Style::default().fg(theme::MUTED),
            )),
            Line::from(""),
        ]);
        lines.extend(enforcement_lines);
    }
    if let Some(message) = &app.rules_ui.detail_message {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                message.clone(),
                Style::default().fg(theme::STATUS_WARN),
            )),
        ]);
    }

    Some(crate::tui::paragraph_max_scroll(&lines, area))
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
            member_ids: vec![id.to_string()],
            severity: severity.to_string(),
            confidence: "high".to_string(),
            category: "convention".to_string(),
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
            match_patterns: Vec::new(),
            lint_bindings: Vec::new(),
            formatter: None,
            tests: Vec::new(),
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
        app.rules_ui.language_filter = crate::tui::app::RulesLanguageFilter::Python;

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
        app.rules_ui.language_filter = crate::tui::app::RulesLanguageFilter::Python;

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
            rendered.contains("fa (Dependency)"),
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
        assert_eq!(source_label(&row), "Personal");

        row.layer = "project".to_string();
        assert_eq!(source_label(&row), "Team");
    }

    #[test]
    fn source_label_maps_non_dependency_rules_to_source() {
        let mut row = mk_row("handbook.write-tests", "should", "project");
        row.source_name = "engineering-handbook".to_string();
        assert_eq!(source_label(&row), "engineering-handbook");
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
        assert!(
            !rendered.contains("No additional fields"),
            "advisory mode should omit placeholder extra fields; got: {}",
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
                    rule_id: format!("custom.grouped-{language}"),
                    description: "Grouped rule".into(),
                    severity: "should".into(),
                    confidence: "high".into(),
                    category: "convention".into(),
                    language: language.into(),
                    source_url: None,
                    dep: Some("custom".into()),
                    enforcement: crate::rule_authoring::EnforcementMode::Pattern {
                        regex: "grouped_rule".into(),
                    },
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

    #[test]
    fn all_filter_only_matches_all_language_rows() {
        let mut all_row = mk_row("custom.all-rule", "should", "personal");
        all_row.languages = vec!["python".into(), "rust".into(), "typescript".into()];
        let python_row = mk_row("custom.python-rule", "should", "personal");
        let data = RulesData {
            rows: vec![all_row, python_row],
        };

        assert_eq!(filtered_rows(&data, RulesLanguageFilter::All).len(), 1);
        assert_eq!(filtered_rows(&data, RulesLanguageFilter::Python).len(), 1);
    }
}
