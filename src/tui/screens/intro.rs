use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::{app::App, components::footer, theme};

const LOGO: &[&str] = &[
    r#"                                         .fvcc]`"#,
    r#"                                       }vvvcccccv1"#,
    r#"                                    ivvvccvcccccccccv."#,
    r#"                                 I\vvvccccccccccccccccu^"#,
    r#"                               (vvcccccvcccccccccccn"vv;"#,
    r#"                            >vvvccccccccccccccccu}]vvvc;"#,
    r#"                         ;tccccccccccccccccccc{+uvccccc;"#,
    r#"                      `}vvcccccccccccccccccui)vccccccc! )\"#,
    r#"                    _uvcccccccccccccccccc?[xvccccccn  ,xvv"#,
    r#"                 .nvccccccccccccccccccc.vcvcccccv_  rvcccc"#,
    r#"                ]?vvcccccccccccccccc)<vccccccc\' "vvvcccc;"#,
    r#"                xccu'ccccccccccccc^xcccccccc>  }vvc/'xc."#,
    r#"                xccccc(l|cccccc(!vccccccc)' Iuvcr""#,
    r#"               )nccccccccxliu;/cccccccc:  (ccc}"#,
    r#"             [t  .fccccccccc'ccccccc/  lvvcf;"#,
    r#"             vvcv(  I/cccccc'cccccI  \vcc<"#,
    r#"             xcccccc-.  xccc'ccx  ^vccn"#,
    r#"              _cccccccc|' `]._  1vcct"#,
    r#"                 .ccccccccx   vvccc|"#,
    r#"                    ~jcccccccvcccc/'"#,
    r#"                       ^cvcccccc<"#,
    r#"                          "|||'"#,
];

const WORDMARK: &[&str] = &[
    "██     ██ ▄▄ ▄▄ ▄▄▄▄▄ ▄▄▄▄▄▄ ▄▄▄▄ ▄▄▄▄▄▄ ▄▄▄  ▄▄  ▄▄ ▄▄▄▄▄",
    "██ ▄█▄ ██ ██▄██ ██▄▄    ██  ███▄▄   ██  ██▀██ ███▄██ ██▄▄",
    " ▀██▀██▀  ██ ██ ██▄▄▄   ██  ▄▄██▀   ██  ▀███▀ ██ ▀██ ██▄▄▄",
];

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[]
}

pub fn render(frame: &mut Frame<'_>, area: Rect, _app: &App) {
    let mut content = Vec::new();
    content.extend(centered_group(LOGO, theme::header_title()));
    content.push(Line::from(""));
    content.push(Line::from(""));
    content.extend(centered_group(WORDMARK, theme::header_title()));
    content.push(Line::from(""));
    content.push(
        Line::from(Span::styled(
            "Press any key to continue",
            Style::default().fg(theme::MUTED),
        ))
        .centered(),
    );

    let top_padding = vertical_padding(area, content.len());
    let mut lines = Vec::with_capacity(top_padding + content.len());
    lines.extend(std::iter::repeat(Line::from("")).take(top_padding));
    lines.extend(content);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_inactive());

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn vertical_padding(area: Rect, content_height: usize) -> usize {
    area.height
        .saturating_sub(2)
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

        assert!(rendered.contains("Press any key to continue"));
        assert!(rendered.contains("██     ██"));
        assert!(rendered.contains("fvcc"));
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

        assert!(joined.contains(r#"`}vvcccccccccccccccccui)vccccccc! )\"#));
        assert!(joined.contains(r#"]?vvcccccccccccccccc)<vccccccc\' "vvvcccc;"#));
        assert!(
            joined.contains(r#"                          "|||'"#) || joined.contains(r#""|||'"#)
        );
    }
}
