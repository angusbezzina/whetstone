//! Top bar: `Whetstone › <screen>   project   v0.4.0`.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::theme;

pub fn render(frame: &mut Frame<'_>, area: Rect, breadcrumb: &str, project_path: &str) {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let line = Line::from(vec![
        Span::styled("Whetstone ", theme::header_title()),
        Span::styled(format!("› {breadcrumb}"), theme::header_title()),
        Span::styled(
            format!("    {project_path}    "),
            theme::header_meta(),
        ),
        Span::styled(version, theme::header_meta()),
    ]);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme::AMBER));

    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}
