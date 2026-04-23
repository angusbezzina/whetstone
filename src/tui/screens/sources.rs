//! Sources screen — subscription manager for committed + personal layers.
//!
//! Scaffolded stub for whetstone-69jb.2. Fill in [`SourcesData`], teach
//! [`load`] to collect both layers via `crate::source_mgmt`, and replace
//! [`render_ready`] with the two-column view.

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
pub enum SourcesView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<SourcesData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct SourcesData {}

pub fn load(_project_dir: &Path) -> SourcesView {
    // TODO(whetstone-69jb.2): gather subscribed sources from both layers.
    SourcesView::Ready(Box::<SourcesData>::default())
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.sources {
        SourcesView::NotComputed => render_placeholder(
            frame,
            area,
            "Sources screen not yet loaded. Press R to compute.",
        ),
        SourcesView::Loading => render_placeholder(frame, area, "Loading sources…"),
        SourcesView::Error(msg) => render_error(frame, area, msg),
        SourcesView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, _data: &SourcesData) {
    // TODO(whetstone-69jb.2): two-column subscription list.
    render_placeholder(frame, area, "Sources screen ready — renderer pending.")
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Sources compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("SOURCES")), area);
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
