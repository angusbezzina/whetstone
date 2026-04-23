//! Dedicated drift screen — shows the post-`wh reinit` walkthrough: which
//! approved rules cite dependencies whose docs have drifted, along with the
//! canned extraction prompt the agent should act on. Data comes from
//! `whetstone/.state/refresh-diff.json`; if the file is missing we show an
//! empty state (not an error — it just means `wh reinit` hasn't run yet).
//!
//! Supports the four data states defined in whetstone-8hm.5.3: not-computed,
//! loading, error, ready. Drift is computed on-disk by `wh reinit` rather
//! than on-demand inside the TUI, so "loading" is effectively never hit here.
//!
//! Items in this module are intentionally `allow(dead_code)` for now: the
//! screen dispatch in `src/tui/mod.rs::view()` still routes `Screen::Drift`
//! through the generic `stub` placeholder. A follow-up bead will add the
//! `DriftView` field to `DashboardState` and wire this module into `view()`;
//! when it does, these attributes come off.
#![allow(dead_code)]

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

pub fn hints() -> &'static [footer::Hint] {
    &[
        ("1", "HOME"),
        ("R", "REFRESH"),
        ("?", "HELP"),
        ("Q", "QUIT"),
    ]
}

/// Four-state view of the drift screen's data. Mirrors `DebtView`.
#[derive(Default, Clone)]
pub enum DriftView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<DriftData>),
    Error(String),
}

#[derive(Clone, Default)]
pub struct DriftData {
    pub generated_at: Option<String>,
    pub candidates: Vec<DriftCandidate>,
    pub extraction_prompt: String,
    pub count: i64,
    pub selected: usize,
}

impl DriftView {
    pub fn select_prev(&mut self) {
        if let DriftView::Ready(data) = self {
            data.selected = data.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if let DriftView::Ready(data) = self {
            let len = data.candidates.len();
            if len > 0 && data.selected + 1 < len {
                data.selected += 1;
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct DriftCandidate {
    pub rule_id: String,
    pub dep: String,
    pub drift_types: Vec<String>,
    pub severity: String,
    pub source_url: String,
}

/// Load drift data from `<project>/whetstone/.state/refresh-diff.json`.
///
/// Missing file → `Ready(default)` so the user sees a helpful empty state
/// rather than a scary error. Malformed JSON → `Error` — that's a real bug.
pub fn load(project_dir: &Path) -> DriftView {
    let path = project_dir
        .join("whetstone")
        .join(".state")
        .join("refresh-diff.json");

    if !path.exists() {
        return DriftView::Ready(Box::default());
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return DriftView::Error(e.to_string()),
    };

    let value: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return DriftView::Error(e.to_string()),
    };

    let generated_at = value
        .get("generated_at")
        .and_then(|v| v.as_str())
        .map(String::from);

    // `count` is the documented field name; `drift_count` is what the emitter
    // in src/handoff.rs actually writes. Accept either.
    let count = value
        .get("count")
        .and_then(|v| v.as_i64())
        .or_else(|| value.get("drift_count").and_then(|v| v.as_i64()))
        .unwrap_or(0);

    let extraction_prompt = value
        .get("extraction_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let candidates = value
        .get("re_extraction_candidates")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| DriftCandidate {
                    rule_id: str_field(c, "rule_id"),
                    dep: str_field(c, "dep"),
                    drift_types: c
                        .get("drift_types")
                        .and_then(|v| v.as_array())
                        .map(|xs| {
                            xs.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    // Handler writes `current_severity`; spec calls it `severity`.
                    severity: c
                        .get("severity")
                        .and_then(|v| v.as_str())
                        .or_else(|| c.get("current_severity").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .to_string(),
                    // Same dual-name story for `source_url`.
                    source_url: c
                        .get("source_url")
                        .and_then(|v| v.as_str())
                        .or_else(|| c.get("current_source_url").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    DriftView::Ready(Box::new(DriftData {
        generated_at,
        candidates,
        extraction_prompt,
        count,
        selected: 0,
    }))
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    // Drift data lives on disk; load it fresh on every render. Cheap — a
    // single small JSON file — and avoids needing extra App state.
    let view = load(&app.project_dir);
    match view {
        DriftView::NotComputed => render_empty(
            frame,
            area,
            "Drift not computed yet. Run `wh reinit` to refresh source docs.",
        ),
        DriftView::Loading => render_empty(frame, area, "Computing drift…"),
        DriftView::Error(msg) => render_error(frame, area, &msg),
        DriftView::Ready(data) if data.candidates.is_empty() => render_ok(frame, area),
        DriftView::Ready(data) => render_ready(frame, area, &data),
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
    frame.render_widget(Paragraph::new(lines).block(block("DRIFT")), area);
}

fn render_ok(frame: &mut Frame<'_>, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  No drift — rules are current. Run wh reinit to check.",
            Style::default().fg(theme::STATUS_OK),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DRIFT")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Drift load failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
        Line::from(""),
        Line::from(Span::styled(
            "  Inspect whetstone/.state/refresh-diff.json and re-run wh reinit.",
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DRIFT")), area);
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &DriftData) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_candidate_list(frame, cols[0], data);
    render_detail_pane(frame, cols[1], data);
}

fn render_candidate_list(frame: &mut Frame<'_>, area: Rect, data: &DriftData) {
    let width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = data
        .candidates
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let marker = if idx == data.selected { "▸ " } else { "  " };
            let drift = if c.drift_types.is_empty() {
                "-".to_string()
            } else {
                c.drift_types.join("+")
            };
            let dep = truncate(&c.dep, width.saturating_sub(32));
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme::AMBER)),
                Span::styled(
                    format!("{} ", c.rule_id),
                    Style::default().fg(theme::AMBER).bold(),
                ),
                Span::styled(
                    format!("[{}] ", c.severity),
                    Style::default().fg(theme::severity_color(&c.severity)),
                ),
                Span::styled(
                    format!("{} ", drift),
                    Style::default().fg(theme::MUTED),
                ),
                Span::raw(dep),
            ]))
        })
        .collect();

    let title = format!(
        "CANDIDATES ({} shown, {} drifted deps)",
        data.candidates.len(),
        data.count
    );
    frame.render_widget(List::new(items).block(block(&title)), area);
}

fn render_detail_pane(frame: &mut Frame<'_>, area: Rect, data: &DriftData) {
    let mut lines: Vec<Line> = Vec::new();

    if !data.extraction_prompt.is_empty() {
        lines.push(Line::from(Span::styled(
            "Extraction prompt",
            theme::header_title(),
        )));
        for chunk in data.extraction_prompt.lines() {
            lines.push(Line::from(Span::styled(
                chunk.to_string(),
                Style::default().fg(theme::MUTED),
            )));
        }
        lines.push(Line::from(""));
    }

    if let Some(candidate) = data.candidates.get(data.selected) {
        lines.push(Line::from(Span::styled(
            "Selected candidate",
            theme::header_title(),
        )));
        lines.push(Line::from(vec![
            Span::styled("  rule_id     ", Style::default().fg(theme::MUTED)),
            Span::styled(
                candidate.rule_id.clone(),
                Style::default().fg(theme::AMBER).bold(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  dep         ", Style::default().fg(theme::MUTED)),
            Span::raw(candidate.dep.clone()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  severity    ", Style::default().fg(theme::MUTED)),
            Span::styled(
                candidate.severity.clone(),
                Style::default().fg(theme::severity_color(&candidate.severity)),
            ),
        ]));
        let drift = if candidate.drift_types.is_empty() {
            "-".to_string()
        } else {
            candidate.drift_types.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("  drift_types ", Style::default().fg(theme::MUTED)),
            Span::raw(drift),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  source_url  ", Style::default().fg(theme::MUTED)),
            Span::raw(candidate.source_url.clone()),
        ]));
    }

    if let Some(ts) = &data.generated_at {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Generated at {ts}"),
            Style::default().fg(theme::MUTED),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block("DETAIL")),
        area,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect()
    }

    fn tmp_project() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "wh_tui_drift_{}_{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn rand_suffix() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn seed_refresh_diff(project: &Path, body: &str) {
        let dir = project.join("whetstone").join(".state");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("refresh-diff.json"), body).unwrap();
    }

    #[test]
    fn render_shows_drift_candidates() {
        let project = tmp_project();
        let body = r#"{
            "generated_at": "2026-04-22T12:00:00Z",
            "drift_count": 2,
            "extraction_prompt": "Re-extract drifted rules",
            "re_extraction_candidates": [
                {
                    "rule_id": "fastapi.async-routes",
                    "dep": "fastapi",
                    "drift_types": ["version", "content_hash"],
                    "current_severity": "should",
                    "current_source_url": "https://fastapi.tiangolo.com/"
                },
                {
                    "rule_id": "pydantic.v2-validators",
                    "dep": "pydantic",
                    "drift_types": ["version"],
                    "current_severity": "must",
                    "current_source_url": "https://docs.pydantic.dev/"
                }
            ]
        }"#;
        seed_refresh_diff(&project, body);

        let app = App::new(&project).unwrap();
        let rendered = render_to_string(&app, 120, 30);
        assert!(
            rendered.contains("fastapi.async-routes"),
            "expected first rule_id; got: {}",
            &rendered[..rendered.len().min(500)]
        );
        assert!(
            rendered.contains("pydantic.v2-validators"),
            "expected second rule_id; got: {}",
            &rendered[..rendered.len().min(500)]
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn render_shows_empty_state() {
        let project = tmp_project();
        // No refresh-diff.json seeded — load() returns Ready(default) with
        // an empty candidates vec, which should render the "No drift" msg.
        let app = App::new(&project).unwrap();
        let rendered = render_to_string(&app, 100, 24);
        assert!(
            rendered.contains("No drift"),
            "expected empty-state message; got: {}",
            &rendered[..rendered.len().min(500)]
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn render_shows_error_message() {
        let project = tmp_project();
        seed_refresh_diff(&project, "{ this is not valid json");
        let app = App::new(&project).unwrap();
        let rendered = render_to_string(&app, 100, 24);
        assert!(
            rendered.contains("Drift load failed"),
            "expected error banner; got: {}",
            &rendered[..rendered.len().min(500)]
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn render_error_variant_directly_shows_message() {
        // Direct test of render_error via a forced Error view — proves the
        // "Error" rendering path surfaces the inner message verbatim.
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_error(frame, frame.area(), "boom"))
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
}
