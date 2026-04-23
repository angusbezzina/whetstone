//! Rules screen — list + detail of merged approved rules.
//!
//! Scaffolded stub for whetstone-69jb.1. Follow the Debt-screen pattern:
//! fill in [`RulesData`] with everything the render needs, teach [`load`]
//! to produce it from `crate::rules` / `crate::layers`, and replace the
//! placeholder body of [`render_ready`] with the real two-pane layout.

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

/// Four data states, identical shape across every second-slice screen.
#[derive(Default, Clone)]
#[allow(dead_code, clippy::enum_variant_names)] // TODO: remove once subagent wires real loader
pub enum RulesView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<RulesData>),
    Error(String),
}

/// Fill in with the fields the renderer needs.
#[derive(Debug, Default, Clone)]
pub struct RulesData {}

/// Synchronously collect the data for this screen. Return `Error(..)` on
/// any fixable failure; `NotComputed` is reserved for "user hasn't opened
/// this screen yet" and should not be produced by `load`.
pub fn load(_project_dir: &Path) -> RulesView {
    // TODO(whetstone-69jb.1): gather merged approved rules via
    // crate::layers::resolve_merged + crate::rules::load_approved_rules.
    RulesView::Ready(Box::<RulesData>::default())
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
        RulesView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, _data: &RulesData) {
    // TODO(whetstone-69jb.1): list/detail layout.
    render_placeholder(frame, area, "Rules screen ready — renderer pending.")
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
