//! Rules screen — list + detail of merged approved rules.
//!
//! First slice for whetstone-69jb.1: static two-pane layout driven by the
//! four-state `RulesView` enum. The left pane lists merged rule ids with a
//! colored severity badge; the right pane shows full detail (description,
//! source_url, layer, language, dep) for the currently selected rule.
//! Keyboard selection is wired via up/down and j/k; the list renders a moving
//! viewport so large rule sets remain navigable.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
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

/// Four data states, identical shape across every second-slice screen.
#[derive(Default, Clone)]
pub enum RulesView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<RulesData>),
    Error(String),
}

/// One row in the left-hand list. Projected from `LayeredRule` so the
/// renderer doesn't touch the underlying `ApprovedRule` shape.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub id: String,
    pub severity: String,
    pub confidence: String,
    pub language: String,
    /// Derived from the rule id prefix (e.g. `fastapi.async-routes` → `fastapi`).
    /// Falls back to the id itself when there is no `.` separator.
    pub dep: String,
    /// `"project"` or `"personal"` — mirrors `layers::Layer::as_str()`.
    pub layer: String,
    pub source_url: String,
    pub description: String,
}

/// Everything the renderer needs. Cheap to clone — mostly short strings.
#[derive(Debug, Default, Clone)]
pub struct RulesData {
    pub rows: Vec<RuleRow>,
    /// `(language, count)` pairs for the header summary. Sorted by language.
    pub by_language: Vec<(String, usize)>,
    /// Selected index into `rows`. v1 pins it to 0; future work wires j/k.
    pub selected: usize,
}

impl RulesView {
    pub fn select_prev(&mut self) {
        if let RulesView::Ready(data) = self {
            data.selected = data.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if let RulesView::Ready(data) = self {
            let len = data.rows.len();
            if len > 0 && data.selected + 1 < len {
                data.selected += 1;
            }
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
                return RulesView::Error(
                    "No rules found — run wh init or wh rule add".into(),
                );
            }
        }
        // Initialized but no approved rules — same actionable hint.
        return RulesView::Error("No rules found — run wh init or wh rule add".into());
    }

    let mut rows: Vec<RuleRow> = merged
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
                language: lr.rule.language.clone(),
                dep,
                layer: lr.layer.as_str().to_string(),
                source_url: lr.rule.source_url.clone(),
                description: lr.rule.description.clone(),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));

    let mut by_lang: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for row in &rows {
        *by_lang.entry(row.language.clone()).or_insert(0) += 1;
    }

    RulesView::Ready(Box::new(RulesData {
        rows,
        by_language: by_lang.into_iter().collect(),
        selected: 0,
    }))
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.rules {
        RulesView::NotComputed => render_placeholder(
            frame,
            area,
            "Rules screen not yet loaded. Press R to compute.",
        ),
        RulesView::Loading => render_placeholder(frame, area, "Loading rules…"),
        RulesView::Error(msg) => render_error(frame, area, msg),
        RulesView::Ready(data) if data.rows.is_empty() => render_placeholder(
            frame,
            area,
            "No approved rules to display.",
        ),
        RulesView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &RulesData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_list(frame, cols[0], data);
    render_detail(frame, cols[1], data);
}

fn render_list(frame: &mut Frame<'_>, area: Rect, data: &RulesData) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = area.height.saturating_sub(2) as usize;
    let (start, end) = window_bounds(data.selected, data.rows.len(), visible);
    let items: Vec<ListItem> = data
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, row)| {
            let marker = if i == data.selected { "▶ " } else { "  " };
            let marker_color = if i == data.selected {
                theme::AMBER
            } else {
                theme::MUTED
            };
            let id_w = width.saturating_sub(16);
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(marker_color)),
                Span::styled(
                    format!("[{}] ", row.severity.to_uppercase()),
                    Style::default().fg(theme::severity_color(&row.severity)).bold(),
                ),
                Span::raw(truncate(&row.id, id_w)),
            ]))
        })
        .collect();

    let title = rules_title(data);
    let list = List::new(items).block(block(&title));
    frame.render_widget(list, area);
}

fn rules_title(data: &RulesData) -> String {
    if data.by_language.is_empty() {
        format!("RULES ({})", data.rows.len())
    } else {
        let breakdown = data
            .by_language
            .iter()
            .map(|(lang, n)| format!("{lang} {n}"))
            .collect::<Vec<_>>()
            .join("  ");
        format!("RULES ({}  ·  {breakdown})", data.rows.len())
    }
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, data: &RulesData) {
    let Some(row) = data.rows.get(data.selected) else {
        render_placeholder(frame, area, "No rule selected.");
        return;
    };

    let layer_color = if row.layer == "personal" {
        theme::AMBER
    } else {
        theme::MUTED
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("id        ", theme::header_meta()),
            Span::styled(row.id.clone(), Style::default().bold()),
        ]),
        Line::from(vec![
            Span::styled("severity  ", theme::header_meta()),
            Span::styled(
                row.severity.to_uppercase(),
                Style::default().fg(theme::severity_color(&row.severity)).bold(),
            ),
            Span::raw("   "),
            Span::styled("confidence ", theme::header_meta()),
            Span::raw(row.confidence.clone()),
        ]),
        Line::from(vec![
            Span::styled("language  ", theme::header_meta()),
            Span::raw(row.language.clone()),
            Span::raw("   "),
            Span::styled("dep ", theme::header_meta()),
            Span::raw(row.dep.clone()),
        ]),
        Line::from(vec![
            Span::styled("layer     ", theme::header_meta()),
            Span::styled(row.layer.clone(), Style::default().fg(layer_color).bold()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "description",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(row.description.clone()),
        Line::from(""),
        Line::from(Span::styled("source", Style::default().fg(theme::MUTED))),
        Line::from(Span::styled(
            row.source_url.clone(),
            Style::default().fg(theme::AMBER),
        )),
    ];

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block("DETAIL"));
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
    frame.render_widget(Paragraph::new(lines).block(block("RULES")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    if msg.contains("No rules found") {
        return render_placeholder(
            frame,
            area,
            "No rules yet. Run wh init, then wh extract and wh approve — or add one with wh rule add.",
        );
    }
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Rules compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("RULES")), area);
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

    fn mk_row(id: &str, severity: &str, layer: &str) -> RuleRow {
        RuleRow {
            id: id.to_string(),
            severity: severity.to_string(),
            confidence: "high".to_string(),
            language: "python".to_string(),
            dep: id.split_once('.').map(|(p, _)| p.to_string()).unwrap_or_default(),
            layer: layer.to_string(),
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
        let tmp = std::env::temp_dir()
            .join(format!("wh_tui_rules_ready_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();

        // Short ids so both fit in the narrow left list pane at 80x24.
        let rows = vec![
            mk_row("fa.async", "must", "project"),
            mk_row("pd.strict", "should", "personal"),
        ];
        let by_language = vec![("python".to_string(), rows.len())];
        app.dashboard.rules = RulesView::Ready(Box::new(RulesData {
            rows,
            by_language,
            selected: 0,
        }));

        let backend = TestBackend::new(80, 24);
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

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn render_shows_error_message() {
        let tmp = std::env::temp_dir()
            .join(format!("wh_tui_rules_error_{}", std::process::id()));
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
}
