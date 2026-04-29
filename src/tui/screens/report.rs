//! Report screen — markdown viewer for `wh report` output.
//!
//! Implemented for whetstone-69jb.5. Calls `crate::report::build` +
//! `crate::report::to_markdown` on first open and caches the rendered
//! markdown. Rendered as plain text through a wrapped, scrollable
//! `Paragraph` — no markdown-to-ratatui formatting in v1.

use std::path::Path;

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
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
pub enum ReportView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<ReportData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct ReportData {
    /// Fully-rendered markdown body from `crate::report::to_markdown`.
    pub markdown: String,
    /// Vertical scroll offset (lines).
    pub scroll_y: u16,
    /// Horizontal scroll offset (columns).
    pub scroll_x: u16,
}

impl ReportView {
    pub fn scroll_up(&mut self, lines: u16) {
        if let ReportView::Ready(data) = self {
            data.scroll_y = data.scroll_y.saturating_sub(lines);
        }
    }

    pub fn scroll_down(&mut self, lines: u16) {
        if let ReportView::Ready(data) = self {
            data.scroll_y = data.scroll_y.saturating_add(lines);
        }
    }

    pub fn scroll_left(&mut self, cols: u16) {
        if let ReportView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_sub(cols);
        }
    }

    pub fn scroll_right(&mut self, cols: u16) {
        if let ReportView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_add(cols);
        }
    }
}

pub fn load(project_dir: &Path) -> ReportView {
    let opts = crate::report::ReportOptions {
        project_dir,
        pr_comment: false,
    };
    match crate::report::build(&opts) {
        Ok(data) => {
            let markdown = crate::report::to_markdown(&data);
            ReportView::Ready(Box::new(ReportData {
                markdown,
                scroll_y: 0,
                scroll_x: 0,
            }))
        }
        Err(e) => ReportView::Error(e.to_string()),
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.report {
        ReportView::NotComputed => render_placeholder(
            frame,
            area,
            "Report not yet generated. Press R to compute.",
        ),
        ReportView::Loading => render_placeholder(frame, area, "Generating report…"),
        ReportView::Error(msg) => render_error(frame, area, msg),
        ReportView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &ReportData) {
    if data.markdown.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Report is empty — run wh scan first to populate violations.",
                Style::default().fg(theme::MUTED),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let paragraph = Paragraph::new(data.markdown.as_str()).scroll((data.scroll_y, data.scroll_x));
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
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Report generation failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect()
    }

    #[test]
    fn render_shows_markdown_body() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir()
            .join(format!("wh_tui_report_ready_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.dashboard.report = ReportView::Ready(Box::new(ReportData {
            markdown: "# Whetstone report\n\nAdherence: 92/100".to_string(),
            scroll_y: 0,
            scroll_x: 0,
        }));
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(
            rendered.contains("Whetstone report"),
            "expected markdown header in buffer; got: {}",
            &rendered[..rendered.len().min(400)]
        );
        assert!(
            rendered.contains("92/100"),
            "expected adherence score in buffer; got: {}",
            &rendered[..rendered.len().min(400)]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn render_shows_error_message() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir()
            .join(format!("wh_tui_report_err_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.dashboard.report = ReportView::Error("boom".into());
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(
            rendered.contains("boom"),
            "expected error message in buffer; got: {}",
            &rendered[..rendered.len().min(400)]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
