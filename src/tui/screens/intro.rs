use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

pub const AUTO_DISMISS_TICKS: u16 = 42;

const TOP_STONE: &[&str] = &[
    "                                            .-.",
    "                                      .=++++++++++++:.",
    "                                .:+++++++***++++++++++++++-.",
    "                          ..=+++**++**+++********+++++**+--+=",
    "                     .:=++++++******++*************+--++++++=",
    "               ..-+++*+++*****************++*++--+++++******=.-:",
    "          ..=++++++++++******++*+++++++*++:-+++++++****+-. .:++++.",
    "      :++++++++++******++**********+=:-++*********+-. .:+++++++*+.",
    "     -*++:-+*******************=.=+++++******+-.. :++++*+++*+-..",
    "     -*******++-:=+*******=:=++*+*******+:. .-+++*+-.",
    " :+-.:+*************+=.=++*********+:...-+++*+-.",
    ".+++*+=:  .:=+*******+:++*****+:. .-++**+-.",
    ".=+****++***=:. .:=+*+:++=:...-++**+-.",
    "     .-+******++**+-.....-++++**+:",
    "            :++**************++:",
    "                  .+*****+.",
];

const BASE_STONE: &[&str] = &[
    "        ╭────────────────────────────────────────────────────────────╮",
    "     ╭──╯                                                            ╰──╮",
    "   ╭─╯                                                                  ╰─╮",
    "   ╰──────────────────────────────────────────────────────────────────────╯",
];

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[]
}

pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let hover_gap = hover_gap(app.intro_ui.frame);
    let loading_text = loading_text(app.intro_ui.frame);
    let top_width = art_width(TOP_STONE);
    let base_width = art_width(BASE_STONE);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.extend(std::iter::repeat(Line::from("")).take(vertical_padding(area.height, hover_gap)));
    lines.extend(TOP_STONE.iter().map(|line| art_line(line, top_width)));
    lines.extend(std::iter::repeat(Line::from("")).take(hover_gap));
    lines.extend(BASE_STONE.iter().map(|line| art_line(line, base_width)));
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(
            "Welcome to Whetstone",
            Style::default().fg(ratatui::style::Color::White).dim(),
        ))
        .centered(),
    );
    lines.push(
        Line::from(Span::styled(
            loading_text,
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
    let content_height = TOP_STONE.len() + BASE_STONE.len() + hover_gap + 3;
    height
        .saturating_sub(2)
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

fn loading_text(frame: u16) -> &'static str {
    match frame % 24 {
        0..=5 => "Loading",
        6..=11 => "Loading.",
        12..=17 => "Loading..",
        _ => "Loading...",
    }
}

fn art_width(lines: &[&str]) -> usize {
    lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

fn art_line(line: &str, width: usize) -> Line<'static> {
    let mut padded = String::from(line);
    padded.extend(std::iter::repeat(' ').take(width.saturating_sub(line.chars().count())));
    Line::from(Span::styled(padded, theme::header_title())).centered()
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

        assert!(rendered.contains("Welcome to Whetstone"));
        assert!(rendered.contains("Loading"));
        assert!(!rendered.contains("gentle hover"));
        assert!(!rendered.contains("Press any key to continue"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hover_gap_cycles_gently() {
        assert_eq!(hover_gap(0), 1);
        assert_eq!(hover_gap(9), 2);
        assert_eq!(hover_gap(16), 0);
    }

    #[test]
    fn loading_text_animates_subtly() {
        assert_eq!(loading_text(0), "Loading");
        assert_eq!(loading_text(6), "Loading.");
        assert_eq!(loading_text(12), "Loading..");
        assert_eq!(loading_text(18), "Loading...");
    }
}
