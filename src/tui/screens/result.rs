//! Generic command-result screen used when a CLI command runs in human TTY mode
//! but does not map cleanly to a dedicated domain screen.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[("1", "HOME"), ("?", "HELP"), ("Q", "QUIT")]
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
    pub scroll_y: u16,
    pub scroll_x: u16,
}

impl ResultView {
    pub fn scroll_up(&mut self, lines: u16) {
        if let ResultView::Ready(data) = self {
            data.scroll_y = data.scroll_y.saturating_sub(lines);
        }
    }

    pub fn scroll_down(&mut self, lines: u16) {
        if let ResultView::Ready(data) = self {
            data.scroll_y = data.scroll_y.saturating_add(lines);
        }
    }

    pub fn scroll_left(&mut self, cols: u16) {
        if let ResultView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_sub(cols);
        }
    }

    pub fn scroll_right(&mut self, cols: u16) {
        if let ResultView::Ready(data) = self {
            data.scroll_x = data.scroll_x.saturating_add(cols);
        }
    }
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
        .scroll((data.scroll_y, data.scroll_x));
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
