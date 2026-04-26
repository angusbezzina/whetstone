//! Generic command-result screen used when a CLI command runs in human TTY mode
//! but does not map cleanly to a dedicated domain screen.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("R", "REFRESH"), ("?", "HELP"), ("Q", "QUIT")]
}

#[derive(Default, Clone)]
pub enum ResultView {
    #[default]
    NotComputed,
    Ready(Box<ResultData>),
}

#[derive(Debug, Clone)]
pub struct ResultData {
    pub title: String,
    pub body: String,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match &app.dashboard.result {
        ResultView::NotComputed => render_placeholder(frame, area, "No command result loaded."),
        ResultView::Ready(data) => render_ready(frame, area, data),
    }
}

fn render_ready(frame: &mut Frame<'_>, area: Rect, data: &ResultData) {
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", data.title),
            theme::header_title(),
        ))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());
    let p = Paragraph::new(data.body.clone())
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn render_placeholder(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let block = Block::default()
        .title(Span::styled(" RESULT ", theme::header_title()))
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
