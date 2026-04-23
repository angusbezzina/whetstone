//! Check screen — violations explorer.
//!
//! Scaffolded stub for whetstone-69jb.4. Fill in [`CheckData`], teach
//! [`load`] to call `crate::check::run` once (share the same CheckOptions
//! path as the dashboard top-5), and replace [`render_ready`].

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
pub enum CheckView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<CheckData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct CheckData {}

pub fn load(_project_dir: &Path) -> CheckView {
    // TODO(whetstone-69jb.4): run wh check and bucket results.
    CheckView::Ready(Box::<CheckData>::default())
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

fn render_ready(frame: &mut Frame<'_>, area: Rect, _data: &CheckData) {
    // TODO(whetstone-69jb.4): grouped violation list + filters.
    render_placeholder(frame, area, "Check screen ready — renderer pending.")
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
