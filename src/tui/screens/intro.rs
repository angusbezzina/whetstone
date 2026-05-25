use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

const LOGO: &[&str] = &[
    r#"██╗    ██╗██╗  ██╗███████╗████████╗███████╗████████╗ ██████╗ ███╗   ██╗███████╗"#,
    r#"██║    ██║██║  ██║██╔════╝╚══██╔══╝██╔════╝╚══██╔══╝██╔═══██╗████╗  ██║██╔════╝"#,
    r#"██║ █╗ ██║███████║█████╗     ██║   ███████╗   ██║   ██║   ██║██╔██╗ ██║█████╗  "#,
    r#"██║███╗██║██╔══██║██╔══╝     ██║   ╚════██║   ██║   ██║   ██║██║╚██╗██║██╔══╝  "#,
    r#"╚███╔███╔╝██║  ██║███████╗   ██║   ███████║   ██║   ╚██████╔╝██║ ╚████║███████╗"#,
    r#" ╚══╝╚══╝ ╚═╝  ╚═╝╚══════╝   ╚═╝   ╚══════╝   ╚═╝    ╚═════╝ ╚═╝  ╚═══╝╚══════╝"#,
];

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, _app: &App) {
    let mut content = Vec::new();
    content.extend(centered_group(
        LOGO,
        Style::default()
            .fg(theme::AMBER)
            .add_modifier(Modifier::BOLD),
    ));
    content.extend(spacer_lines(area.height, LOGO.len() + 1));
    content.push(
        Line::from(Span::styled(
            "Press enter to start",
            Style::default().fg(theme::MUTED),
        ))
        .centered(),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::AMBER));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let top_padding = vertical_padding(inner.height, content.len());
    let mut lines = Vec::with_capacity(top_padding + content.len());
    lines.extend(std::iter::repeat(Line::from("")).take(top_padding));
    lines.extend(content);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn spacer_lines(height: u16, minimum_content_height: usize) -> impl Iterator<Item = Line<'static>> {
    let spacer_count = if height as usize > minimum_content_height {
        1
    } else {
        0
    };
    std::iter::repeat(Line::from("")).take(spacer_count)
}

fn vertical_padding(height: u16, content_height: usize) -> usize {
    height
        .saturating_sub(content_height as u16)
        .saturating_div(2) as usize
}

fn centered_group(lines: &[&str], style: Style) -> Vec<Line<'static>> {
    let common_indent = common_indent(lines);
    let cropped: Vec<String> = lines
        .iter()
        .map(|line| strip_indent(line, common_indent).trim_end().to_string())
        .collect();
    let width = cropped
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);

    cropped
        .into_iter()
        .map(|mut line| {
            line.extend(std::iter::repeat(' ').take(width.saturating_sub(line.chars().count())));
            Line::from(Span::styled(line, style)).centered()
        })
        .collect()
}

fn common_indent(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|c| *c == ' ').count())
        .min()
        .unwrap_or(0)
}

fn strip_indent(line: &str, indent: usize) -> &str {
    if indent == 0 {
        return line;
    }

    line.char_indices()
        .nth(indent)
        .map(|(idx, _)| &line[idx..])
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::msg::Screen;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn intro_render_contains_static_branding() {
        let tmp = std::env::temp_dir().join(format!("wh_intro_render_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Intro;

        let backend = TestBackend::new(100, 36);
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

        assert!(rendered.contains("Press enter to start"));
        assert!(rendered.contains("██╗    ██╗"));
        assert!(rendered.contains("███████╗"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("┘"));
        assert!(!rendered.contains("Press any key to continue"));
        assert!(!rendered.contains("fvcc"));
        assert!(!rendered.contains("gentle hover"));
        assert!(!rendered.contains("Loading"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn logo_group_preserves_source_shape() {
        let lines = centered_group(LOGO, Style::default());
        let joined = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("██╗    ██╗██╗  ██╗"));
        assert!(joined.contains("╚███╔███╔╝██║  ██║"));
        assert!(joined.contains("╚═══╝╚══════╝"));
    }

    #[test]
    fn intro_content_fits_twenty_four_rows() {
        let tmp = std::env::temp_dir().join(format!("wh_intro_24_rows_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Intro;

        let backend = TestBackend::new(100, 24);
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

        assert!(rendered.contains("██╗    ██╗"));
        assert!(rendered.contains("Press enter to start"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
