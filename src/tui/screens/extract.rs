//! Extract screen — worklist + bundle review.
//!
//! Scaffolded stub for whetstone-69jb.3. Fill in [`ExtractData`], teach
//! [`load`] to collect the ranked worklist + selected-dep context via
//! `crate::worklist` / `crate::extract`, and replace [`render_ready`].

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
pub enum ExtractView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<ExtractData>),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct ExtractData {}

pub fn load(_project_dir: &Path) -> ExtractView {
    // TODO(whetstone-69jb.3): collect worklist + selected dep details.
    ExtractView::Ready(Box::<ExtractData>::default())
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.extract {
        ExtractView::NotComputed => render_placeholder(
            frame,
            area,
            "Extract screen not yet loaded. Press R to compute.",
        ),
        ExtractView::Loading => render_placeholder(frame, area, "Loading worklist…"),
        ExtractView::Error(msg) => render_error(frame, area, msg),
        ExtractView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, _data: &ExtractData) {
    // TODO(whetstone-69jb.3): worklist + detail layout.
    render_placeholder(frame, area, "Extract screen ready — renderer pending.")
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("EXTRACT")), area);
}

fn render_error(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Extract compute failed:",
            Style::default().fg(theme::STATUS_WARN),
        )),
        Line::from(format!("  {msg}")),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("EXTRACT")), area);
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
