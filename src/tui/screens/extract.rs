//! Dedicated rule extraction screen — per-package extraction utility view with
//! a detailed explanation of why a dependency is or is not a strong next rule
//! extraction target.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

use crate::tui::{app::App, components::footer, theme};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
}

#[derive(Default, Clone)]
pub enum ExtractView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<ExtractData>),
    Error(String),
}

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

#[derive(Debug, Clone)]
pub struct WorklistRow {
    pub name: String,
    pub language: String,
    pub priority: String,
    pub utility_percent: u8,
    pub next_step: String,
    pub remaining_quota: u32,
    pub source_type: Option<String>,
    pub source_url: Option<String>,
    pub version: Option<String>,
    pub registry: Option<String>,
    pub freshness_confidence: Option<String>,
    pub source_age_days: Option<i64>,
    pub reason: Option<String>,
    pub sections: Vec<SectionRow>,
}

#[derive(Debug, Clone)]
pub struct SectionRow {
    pub name: String,
    pub kind: String,
    pub url: String,
    pub bytes: u64,
}

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
    handoff
        .get("worklist")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|e| WorklistRow {
            name: str_field(e, "name"),
            language: str_field(e, "language"),
            priority: str_field(e, "priority"),
            utility_percent: parse_utility_percent(
                e,
                e.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
            ),
            next_step: str_field(e, "next_step"),
            remaining_quota: e
                .get("quota")
                .and_then(|v| v.get("remaining"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            source_type: opt_str_field(e, "source_type"),
            source_url: opt_str_field(e, "source_url"),
            version: opt_str_field(e, "version"),
            registry: opt_str_field(e, "registry"),
            freshness_confidence: e
                .get("freshness")
                .and_then(|v| v.get("confidence"))
                .and_then(|v| v.as_str())
                .map(String::from),
            source_age_days: e
                .get("freshness")
                .and_then(|v| v.get("source_age_days"))
                .and_then(|v| v.as_i64()),
            reason: opt_str_field(e, "reason"),
            sections: e
                .get("sections")
                .and_then(|v| v.as_array())
                .map(|sections| sections.iter().map(section_row).collect())
                .unwrap_or_default(),
        })
        .collect()
}

fn section_row(section: &Value) -> SectionRow {
    let raw_name = section
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("section");
    let raw_kind = section
        .get("source_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("document");
    let mut kind = theme::humanize_token(raw_kind);
    let name = theme::humanize_token(raw_name);
    if kind == name {
        kind = "Document".to_string();
    }
    let bytes = section.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    let url = section
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("no linked section")
        .to_string();

    SectionRow {
        name,
        kind,
        url,
        bytes,
    }
}

fn str_field(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn opt_str_field(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(|v| v.as_str()).map(String::from)
}

#[allow(dead_code)]
pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.extract {
        ExtractView::NotComputed => {
            render_empty(frame, area, "Internal sources are not loaded yet.")
        }
        ExtractView::Loading => render_empty(frame, area, "Loading internal sources…"),
        ExtractView::Error(msg) => render_error(frame, area, msg),
        ExtractView::Ready(data) if data.entries.is_empty() => render_empty(
            frame,
            area,
            "No internal sources are available right now. Run wh init to generate them.",
        ),
        ExtractView::Ready(data) => render_ready(frame, area, data),
    }
}

#[allow(dead_code)]
fn render_empty(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(block("INTERNAL SOURCES", false)),
        area,
    );
}

#[allow(dead_code)]
fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Internal sources failed to load:",
            Style::default().fg(theme::STATUS_WARN),
        )),
    ];
    for part in msg.lines() {
        lines.push(Line::from(format!("  {part}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Exit and reopen the TUI to retry.",
        Style::default().fg(theme::MUTED),
    )));

    frame.render_widget(
        Paragraph::new(lines).block(block("INTERNAL SOURCES", false)),
        area,
    );
}

#[allow(dead_code)]
fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(area);

    render_worklist(frame, cols[0], data);
    render_detail(frame, cols[1], data);
}

pub fn render_worklist(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
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
            let utility_color = theme::utility_color(row.utility_percent);
            let name_style = if is_selected {
                Style::default().fg(theme::AMBER).bold()
            } else {
                Style::default()
            };

            let title_line = Line::from(vec![
                Span::styled(format!("{rank:>3}.  "), Style::default().fg(theme::MUTED)),
                Span::styled(truncate(&row.name, width.saturating_sub(14)), name_style),
            ]);
            let mut meta_spans = vec![Span::styled(
                format!("{:<12}", display_language(&row.language)),
                Style::default().fg(theme::MUTED),
            )];
            if let Some(source_type) = &row.source_type {
                meta_spans.push(Span::raw(" · "));
                meta_spans.push(Span::styled(
                    theme::humanize_token(source_type),
                    Style::default().fg(utility_color),
                ));
            }
            let meta_line = Line::from(meta_spans);

            ListItem::new(vec![title_line, meta_line])
        })
        .collect();

    let title = format!("CORE PACKAGES ({} total)", data.total);
    frame.render_widget(List::new(items).block(block(&title, false)), area);
}

pub fn render_detail(frame: &mut Frame<'_>, area: Rect, data: &ExtractData) {
    render_detail_scrolled(frame, area, data, 0, false);
}

pub fn detail_max_scroll(area: Rect, data: &ExtractData) -> u16 {
    let Some(row) = data.entries.get(data.selected) else {
        return 0;
    };
    crate::tui::paragraph_max_scroll(&detail_lines(area.width, row), area)
}

pub fn render_detail_scrolled(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &ExtractData,
    scroll_y: u16,
    active: bool,
) {
    let Some(row) = data.entries.get(data.selected) else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No package selected.",
                Style::default().fg(theme::MUTED),
            )))
            .block(block("DETAIL", active)),
            area,
        );
        return;
    };

    let lines = detail_lines(area.width, row);

    let effective_scroll = scroll_y.min(crate::tui::paragraph_max_scroll(&lines, area));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("DETAIL", active))
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0)),
        area,
    );
}

fn detail_lines(width: u16, row: &WorklistRow) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = vec![
        kv_line(
            width,
            "Package",
            &row.name,
            Style::default().fg(theme::AMBER).bold(),
        ),
        detail_line(width, "Language", &display_language(&row.language)),
        kv_line(
            width,
            "Utility",
            utility_label(row.utility_percent),
            Style::default()
                .fg(theme::utility_color(row.utility_percent))
                .bold(),
        ),
        detail_line(width, "Recommendation", recommendation_label(&row.priority)),
        detail_line(width, "Rules", &row.remaining_quota.to_string()),
    ];

    let mut source_quality: Vec<Line> = Vec::new();

    if let Some(source_type) = &row.source_type {
        source_quality.push(detail_line(
            width,
            "Source Type",
            &theme::humanize_token(source_type),
        ));
    }
    if let Some(version) = &row.version {
        source_quality.push(detail_line(width, "Version", version));
    }
    if let Some(registry) = &row.registry {
        source_quality.push(detail_line(
            width,
            "Registry",
            &theme::humanize_token(registry),
        ));
    }
    if let Some(confidence) = &row.freshness_confidence {
        let mut value = theme::humanize_token(confidence);
        if let Some(days) = row.source_age_days {
            value.push_str(&format!(" ({days}d old)"));
        }
        source_quality.push(detail_line(width, "Freshness", &value));
    }
    if let Some(url) = &row.source_url {
        source_quality.push(detail_line(width, "Docs", &truncate(url, 88)));
    }
    if let Some(reason) = &row.reason {
        source_quality.push(detail_line(width, "Constraint", reason));
    }
    if !source_quality.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Source quality",
            theme::header_title(),
        )));
        lines.push(Line::from(""));
        lines.extend(source_quality);
    }

    if !row.sections.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Available source material",
            theme::header_title(),
        )));
        lines.push(Line::from(""));
        for section in row.sections.iter().take(5) {
            lines.push(Line::from(format!(
                "{} / {} ({}) [{} bytes]",
                section.name, section.kind, section.url, section.bytes
            )));
        }
        if row.sections.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("… +{} more sections", row.sections.len() - 5),
                Style::default().fg(theme::MUTED),
            )));
        }
    }

    let reasons = utility_reasons(row);
    if !reasons.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Why this can produce rules",
            theme::header_title(),
        )));
        lines.push(Line::from(""));
        for reason in reasons {
            lines.push(Line::from(Span::styled(
                reason,
                Style::default().fg(theme::MUTED),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Next step", theme::header_title())));
    lines.push(Line::from(""));
    lines.push(Line::from(row.next_step.clone()));

    lines
}

fn utility_label(percent: u8) -> &'static str {
    if percent >= 80 {
        "High"
    } else if percent >= 50 {
        "Moderate"
    } else {
        "Low"
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

fn utility_reasons(row: &WorklistRow) -> Vec<String> {
    let mut reasons = Vec::new();

    reasons.push(match row.priority.as_str() {
        "ready_now" => {
            "Documentation resolved cleanly enough that this package is ready for rule extraction now."
                .to_string()
        }
        "resolved_low" => {
            "Documentation was resolved, but the source quality is weaker, so only well-cited rules should be extracted."
                .to_string()
        }
        "pending" => {
            "Source resolution is still pending, so utility is limited until Whetstone finds a better source."
                .to_string()
        }
        "failed" => {
            "Source resolution failed, so this package cannot produce reliable rules yet."
                .to_string()
        }
        "skipped" => "This package is currently skipped by configuration filters.".to_string(),
        _ => {
            "This package needs more source context before it becomes a strong extraction target."
                .to_string()
        }
    });

    if let Some(source_type) = &row.source_type {
        reasons.push(format!(
            "Primary source type is {}.",
            theme::humanize_token(source_type)
        ));
    }

    if let Some(confidence) = &row.freshness_confidence {
        let mut freshness = format!(
            "Freshness confidence is {}",
            theme::humanize_token(confidence)
        );
        if let Some(days) = row.source_age_days {
            freshness.push_str(&format!(" and the source is about {days} day(s) old"));
        }
        freshness.push('.');
        reasons.push(freshness);
    }

    if !row.sections.is_empty() {
        reasons.push(format!(
            "Whetstone captured {} structured source section(s) for this package.",
            row.sections.len()
        ));
    } else {
        reasons.push(
            "No structured source sections were captured, so every proposed rule will need careful doc verification."
                .to_string(),
        );
    }

    if row.remaining_quota == 0 {
        reasons.push("There is no remaining per-package rule quota, so new rules would need existing rules to be removed or rebalanced.".to_string());
    } else {
        reasons.push(format!(
            "There are {} remaining rule slot(s) for this package.",
            row.remaining_quota
        ));
    }

    reasons
}

fn recommendation_label(priority: &str) -> &'static str {
    match priority {
        "ready_now" => "Extract Now",
        "resolved_low" => "Review Source First",
        "pending" => "Await Better Source",
        "failed" => "Fix Source Resolution",
        "skipped" => "Skipped by Config",
        _ => "Needs Review",
    }
}

fn display_language(language: &str) -> String {
    if language.trim().is_empty() {
        "Unknown".to_string()
    } else {
        theme::humanize_token(language)
    }
}

fn parse_utility_percent(entry: &Value, score: f64) -> u8 {
    entry
        .get("utility_percent")
        .and_then(|v| v.as_u64().map(|n| n as u8))
        .or_else(|| {
            entry
                .get("utility_percent")
                .and_then(|v| v.as_f64())
                .map(|n| n.round().clamp(0.0, 100.0) as u8)
        })
        .unwrap_or_else(|| utility_from_score(score))
}

fn utility_from_score(score: f64) -> u8 {
    ((score / 140.0) * 100.0).round().clamp(0.0, 100.0) as u8
}

fn detail_line(width: u16, label: &str, value: &str) -> Line<'static> {
    kv_line(
        width,
        label,
        value,
        Style::default().fg(ratatui::style::Color::White),
    )
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
        let tmp = std::env::temp_dir().join(format!("wh_tui_extract_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        App::new(&tmp).expect("App::new")
    }

    #[test]
    fn render_shows_worklist_entries() {
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = synthetic_app();
        app.dashboard.extract = ExtractView::Ready(Box::new(ExtractData {
            entries: vec![
                WorklistRow {
                    name: "fastapi".into(),
                    language: "python".into(),
                    priority: "ready_now".into(),
                    utility_percent: 89,
                    next_step: "Read the linked source, draft up to 3 rule(s)".into(),
                    remaining_quota: 3,
                    source_type: Some("llms_full_txt".into()),
                    source_url: Some("https://fastapi.tiangolo.com".into()),
                    version: Some("1.0.0".into()),
                    registry: Some("pypi".into()),
                    freshness_confidence: Some("high".into()),
                    source_age_days: Some(12),
                    reason: None,
                    sections: vec![SectionRow {
                        name: "Guide".into(),
                        kind: "Document".into(),
                        url: "https://example.com/guide".into(),
                        bytes: 1200,
                    }],
                },
                WorklistRow {
                    name: "react".into(),
                    language: "typescript".into(),
                    priority: "resolved_low".into(),
                    utility_percent: 55,
                    next_step: "Source is llms_txt — proceed carefully".into(),
                    remaining_quota: 5,
                    source_type: Some("llms_txt".into()),
                    source_url: Some("https://react.dev".into()),
                    version: Some("19.0.0".into()),
                    registry: Some("npm".into()),
                    freshness_confidence: Some("medium".into()),
                    source_age_days: Some(48),
                    reason: None,
                    sections: vec![],
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

        assert!(rendered.contains("CORE PACKAGES"));
        assert!(rendered.contains("Llms Full Txt") || rendered.contains("LLMS"));
        assert!(rendered.contains("fastapi"));
        assert!(rendered.contains("react"));
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

        assert!(rendered.contains("boom"));
    }

    #[test]
    fn load_missing_handoff_returns_error() {
        let tmp =
            std::env::temp_dir().join(format!("wh_tui_extract_missing_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let view = load(&tmp);
        match view {
            ExtractView::Error(msg) => {
                assert!(msg.contains("wh init") || msg.contains("wh reinit"));
            }
            _ => panic!("expected ExtractView::Error when handoff is missing"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn utility_percent_falls_back_to_score_when_missing() {
        let entry = serde_json::json!({
            "score": 125.0,
        });
        assert_eq!(parse_utility_percent(&entry, 125.0), 89);
    }

    #[test]
    fn utility_band_labels_are_human_readable() {
        assert_eq!(utility_label(90), "High");
        assert_eq!(utility_label(65), "Moderate");
        assert_eq!(utility_label(20), "Low");
    }
}
