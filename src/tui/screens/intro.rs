use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

pub const AUTO_DISMISS_TICKS: u16 = 42;

const TOP_STONE: &[&str] = &[
    "              ╱────────────────────────────╲",
    "             ╱──────────────────────────────╲",
    "            ╱________________________________╲",
    "            │                                ││",
    "            │                                ││",
    "            │                                │╱",
    "            ╲________________________________╱",
];

const BASE_STONE: &[&str] = &[
    "         ╭──────────────────────────────────────╮",
    "      ╭──╯                                      ╰──╮",
    "    ╭─╯                                          ╰─╮",
    "    ╰──────────────────────────────────────────────╯",
];

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[]
}

pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let hover_gap = hover_gap(app.intro_ui.frame);
    let shadow_text = shadow_text(app.intro_ui.frame);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.extend(std::iter::repeat(Line::from("")).take(vertical_padding(area.height, hover_gap)));
    lines.extend(TOP_STONE.iter().map(|line| {
        Line::from(Span::styled((*line).to_string(), theme::header_title())).centered()
    }));
    lines.extend(std::iter::repeat(Line::from("")).take(hover_gap));
    lines.extend(BASE_STONE.iter().map(|line| {
        Line::from(Span::styled((*line).to_string(), theme::header_title())).centered()
    }));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(shadow_text, Style::default().fg(theme::MUTED))).centered());
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("WHETSTONE", theme::header_title())).centered());
    lines.push(
        Line::from(Span::styled(
            "Sharpen the tools that write your code",
            Style::default().fg(ratatui::style::Color::White).dim(),
        ))
        .centered(),
    );
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(
            "Press any key to continue · auto-starting shortly",
            Style::default().fg(theme::MUTED),
        ))
        .centered(),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn vertical_padding(height: u16, hover_gap: usize) -> usize {
    let content_height = TOP_STONE.len() + BASE_STONE.len() + hover_gap + 6;
    height
        .saturating_sub(content_height as u16)
        .saturating_div(2) as usize
}

fn hover_gap(frame: u16) -> usize {
    match frame % 24 {
        0..=7 => 1,
        8..=11 => 2,
        12..=13 => 1,
        14..=19 => 0,
        _ => 1,
    }
}

fn shadow_text(frame: u16) -> String {
    match frame % 16 {
        0..=3 => "                ╲  gentle hover  ╱".to_string(),
        4..=7 => "                 ╲ gentle hover ╱ ".to_string(),
        8..=11 => "                  ╲gentle hover╱  ".to_string(),
        _ => "                 ╱ gentle hover ╲ ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::msg::Screen;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn intro_render_contains_branding() {
        let tmp = std::env::temp_dir().join(format!("wh_intro_render_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Intro;

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();

        assert!(rendered.contains("WHETSTONE"));
        assert!(rendered.contains("Press any key to continue"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hover_gap_cycles_gently() {
        assert_eq!(hover_gap(0), 1);
        assert_eq!(hover_gap(9), 2);
        assert_eq!(hover_gap(16), 0);
    }
}
