//! Check screen — violations explorer.
//!
//! Runs `crate::check::run` against the same scan root the dashboard uses
//! and displays every violation in a scrollable list grouped by severity.

use std::path::Path;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
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
pub enum CheckView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<CheckData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct CheckData {
    pub violations: Vec<ViolationRow>,
    pub counts: ViolationCounts,
    pub files_scanned: u32,
    pub rules_applied: u32,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct ViolationRow {
    pub rule_id: String,
    pub severity: String,
    pub file: String,
    pub line: u64,
    pub snippet: String,
}

#[derive(Debug, Default, Clone)]
pub struct ViolationCounts {
    pub must: u32,
    pub should: u32,
    pub may: u32,
}

impl CheckView {
    pub fn select_prev(&mut self) {
        if let CheckView::Ready(data) = self {
            data.selected = data.selected.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if let CheckView::Ready(data) = self {
            let len = data.violations.len();
            if len > 0 && data.selected + 1 < len {
                data.selected += 1;
            }
        }
    }
}

pub fn load(project_dir: &Path) -> CheckView {
    // Match what `collect_dashboard` does in src/tui/app.rs: prefer `src/`
    // when present, otherwise scan from the project root.
    let scan_root = if project_dir.join("src").is_dir() {
        project_dir.join("src")
    } else {
        project_dir.to_path_buf()
    };

    let opts = crate::check::CheckOptions {
        project_dir,
        scan_paths: std::slice::from_ref(&scan_root),
        lang_filter: None,
        rule_filter: None,
    };

    let value = match crate::check::run(opts) {
        Ok(v) => v,
        Err(e) => return CheckView::Error(e.to_string()),
    };

    let files_scanned = value
        .get("files_scanned")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let rules_applied = value
        .get("rules_applied")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut violations: Vec<ViolationRow> = value
        .get("violations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(ViolationRow {
                        rule_id: v.get("rule_id").and_then(|s| s.as_str())?.to_string(),
                        severity: v.get("severity").and_then(|s| s.as_str())?.to_string(),
                        file: v.get("file").and_then(|s| s.as_str())?.to_string(),
                        line: v.get("line").and_then(|s| s.as_u64()).unwrap_or(0),
                        snippet: v
                            .get("match")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Sort: must → should → may → anything else.
    violations.sort_by_key(|v| match v.severity.as_str() {
        "must" => 0,
        "should" => 1,
        "may" => 2,
        _ => 3,
    });

    let mut counts = ViolationCounts::default();
    for v in &violations {
        match v.severity.as_str() {
            "must" => counts.must += 1,
            "should" => counts.should += 1,
            "may" => counts.may += 1,
            _ => {}
        }
    }

    CheckView::Ready(Box::new(CheckData {
        violations,
        counts,
        files_scanned,
        rules_applied,
        selected: 0,
    }))
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.check {
        CheckView::NotComputed => render_placeholder(
            frame,
            area,
            "Check screen not yet loaded. Press R to compute.",
        ),
        CheckView::Loading => render_placeholder(frame, area, "Running check…"),
        CheckView::Error(msg) => render_error(frame, area, msg),
        CheckView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &CheckData) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    render_summary(frame, rows[0], data);
    render_violations(frame, rows[1], data);
}

fn render_summary(frame: &mut Frame<'_>, area: Rect, data: &CheckData) {
    let total = data.violations.len();
    let line = Line::from(vec![
        Span::styled(
            format!("{total} violations"),
            Style::default().fg(theme::AMBER).bold(),
        ),
        Span::raw(" ("),
        Span::styled("must ", Style::default().fg(theme::SEVERITY_MUST)),
        Span::raw(format!("{}", data.counts.must)),
        Span::raw(", "),
        Span::styled("should ", Style::default().fg(theme::SEVERITY_SHOULD)),
        Span::raw(format!("{}", data.counts.should)),
        Span::raw(", "),
        Span::styled("may ", Style::default().fg(theme::SEVERITY_MAY)),
        Span::raw(format!("{}", data.counts.may)),
        Span::raw(")"),
        Span::styled("  ·  ", Style::default().fg(theme::MUTED)),
        Span::raw(format!("{} rules applied", data.rules_applied)),
        Span::styled("  ·  ", Style::default().fg(theme::MUTED)),
        Span::raw(format!("{} files scanned", data.files_scanned)),
    ]);
    frame.render_widget(Paragraph::new(line).block(block("SUMMARY")), area);
}

fn render_violations(frame: &mut Frame<'_>, area: Rect, data: &CheckData) {
    let width = area.width.saturating_sub(4) as usize;
    let visible = area.height.saturating_sub(2) as usize;
    let (start, end) = window_bounds(data.selected, data.violations.len(), visible);

    let items: Vec<ListItem> = if data.violations.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No violations. Nice.",
            Style::default().fg(theme::STATUS_OK),
        )))]
    } else {
        data.violations
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|(i, v)| {
                let sev_color = theme::severity_color(&v.severity);
                let location = format!("{}:{}", v.file, v.line);
                // Mirror dashboard's render_violations_panel column widths,
                // but adapt snippet width to the area we actually have.
                let snippet_w = width
                    .saturating_sub(7 + 30 + 28 + 3)
                    .max(10);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if i == data.selected { "▶" } else { " " },
                        Style::default().fg(if i == data.selected { theme::AMBER } else { theme::MUTED }),
                    ),
                    Span::styled(
                        format!("{:<7}", v.severity.to_uppercase()),
                        Style::default().fg(sev_color).bold(),
                    ),
                    Span::styled(
                        format!("{:<30} ", truncate(&v.rule_id, 30)),
                        Style::default().fg(theme::AMBER),
                    ),
                    Span::raw(format!("{:<28} ", truncate(&location, 28))),
                    Span::styled(
                        truncate(&v.snippet, snippet_w),
                        Style::default().fg(theme::MUTED),
                    ),
                ]))
            })
            .collect()
    };

    let title = if data.violations.is_empty() {
        "VIOLATIONS".to_string()
    } else {
        format!("VIOLATIONS ({} total)", data.violations.len())
    };
    let list = List::new(items).block(block(&title));
    frame.render_widget(list, area);
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("CHECK")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Check compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("CHECK")), area);
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
    use ratatui::{backend::TestBackend, Terminal};

    fn temp_project() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "wh_tui_check_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn row(rule_id: &str, severity: &str, file: &str, line: u64, snippet: &str) -> ViolationRow {
        ViolationRow {
            rule_id: rule_id.into(),
            severity: severity.into(),
            file: file.into(),
            line,
            snippet: snippet.into(),
        }
    }

    #[test]
    fn render_shows_violations() {
        let tmp = temp_project();
        let mut app = App::new(&tmp).unwrap();
        app.screen = crate::tui::msg::Screen::Check;
        app.dashboard.check = CheckView::Ready(Box::new(CheckData {
            violations: vec![
                row("fastapi.async-routes", "must", "src/app.py", 10, "def foo():"),
                row("fastapi.response-model", "should", "src/routes.py", 42, "return {}"),
            ],
            counts: ViolationCounts {
                must: 1,
                should: 1,
                may: 0,
            },
            files_scanned: 3,
            rules_applied: 7,
            selected: 0,
        }));

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
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
            rendered.contains("fastapi.async-routes"),
            "expected first rule_id in output; got: {}",
            &rendered[..rendered.len().min(600)]
        );
        assert!(
            rendered.contains("fastapi.response-model"),
            "expected second rule_id in output; got: {}",
            &rendered[..rendered.len().min(600)]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn render_shows_error_message() {
        let tmp = temp_project();
        let mut app = App::new(&tmp).unwrap();
        app.screen = crate::tui::msg::Screen::Check;
        app.dashboard.check = CheckView::Error("boom".into());

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
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
            "expected error message in output; got: {}",
            &rendered[..rendered.len().min(400)]
        );
        assert!(rendered.contains("compute failed"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
