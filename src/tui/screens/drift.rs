//! Drift screen — re-extraction candidates from `.state/refresh-diff.json`.
//!
//! Scaffolded stub for whetstone-69jb.6. Fill in [`DriftData`], teach
//! [`load`] to read refresh-diff.json, and replace [`render_ready`] with
//! a two-pane view (candidate list + canned extraction prompt).

use std::path::Path;

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
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
#[allow(dead_code, clippy::enum_variant_names)] // TODO: remove once subagent wires real loader
pub enum DriftView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<DriftData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct DriftData {}

pub fn load(_project_dir: &Path) -> DriftView {
    // TODO(whetstone-69jb.6): read refresh-diff.json and project into DriftData.
    DriftView::Ready(Box::<DriftData>::default())
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.drift {
        DriftView::NotComputed => render_placeholder(
            frame,
            area,
            "Drift screen not yet loaded. Press R to compute.",
        ),
        DriftView::Loading => render_placeholder(frame, area, "Loading drift…"),
        DriftView::Error(msg) => render_error(frame, area, msg),
        DriftView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, _data: &DriftData) {
    // TODO(whetstone-69jb.6): candidates + canned extraction prompt.
    render_placeholder(frame, area, "Drift screen ready — renderer pending.")
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DRIFT")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Drift compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("DRIFT")), area);
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
