//! Whetstone TUI — interactive dashboard for Epic 4A.
//!
//! Invoked by a bare `wh` on a TTY, or used as the interactive wrapper for
//! human-friendly command runs. Elm-style loop:
//! `Terminal::draw(view) → event::read() → Msg → App::update(Msg) → loop`.
//! Screens live under [`screens`]; reusable widgets under [`components`].

use std::io;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

pub mod app;
pub mod components;
pub mod msg;
pub mod screens;
pub mod theme;

use app::App;
use components::{footer, header};
use msg::{Msg, Screen};

pub enum LaunchTarget {
    Screen(Screen),
    Result { title: String, body: String },
}

/// Minimum usable terminal size. Below this we render a "please resize" notice.
const MIN_WIDTH: u16 = 50;
const MIN_HEIGHT: u16 = 15;
const NORMAL_TICK_MS: u64 = 100;

/// Check whether stdout is a TTY. `wh` with no args uses this to decide
/// whether to launch the TUI or dump the CLI help.
pub fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    io::stdout().is_terminal()
}

/// Blocking entry point. Sets up the terminal, runs the main loop, restores.
pub fn run(project_dir: &Path) -> Result<()> {
    let mut app = App::new(project_dir).context("failed to load project data")?;
    app.start_intro();

    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

pub fn run_with_target(project_dir: &Path, target: LaunchTarget) -> Result<()> {
    let mut app = App::new(project_dir).context("failed to load project data")?;
    match target {
        LaunchTarget::Screen(screen) => {
            app.screen = screen;
            app.ensure_current_screen_loaded();
        }
        LaunchTarget::Result { title, body } => {
            app.screen = Screen::Result;
            app.dashboard.result =
                screens::result::ResultView::Ready(Box::new(screens::result::ResultData {
                    title,
                    body,
                    scroll_y: 0,
                    scroll_x: 0,
                }));
        }
    }

    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alt screen")?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("failed to init terminal")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    if is_raw_mode_enabled().unwrap_or(false) {
        let _ = disable_raw_mode();
    }
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| view(frame, app))?;

        if event::poll(Duration::from_millis(NORMAL_TICK_MS))? {
            match event::read()? {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                    app.update(Msg::Key(key));
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

/// Root view — splits the frame into header / body / footer and dispatches
/// the body to the active screen.
pub fn view(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    if app.screen == Screen::Intro {
        screens::render(frame, area, app);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(0),    // body
            Constraint::Length(2), // footer
        ])
        .split(area);

    let breadcrumb = app.screen.title();
    let project = app.project_dir.display().to_string();
    header::render(frame, chunks[0], breadcrumb, &project);

    let body = chunks[1];
    let hints: &[footer::Hint] = footer::global_hints();

    screens::render(frame, body, app);

    if app.input_mode == app::InputMode::Normal {
        let scroll_hint = screens::scroll_hint(app.screen, body, app);
        footer::render(frame, chunks[2], hints, scroll_hint);
    } else {
        footer::render_form(frame, chunks[2], hints);
    }
}

pub(crate) fn paragraph_max_scroll(lines: &[Line<'_>], area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2) as usize;
    let viewport_lines = area.height.saturating_sub(2) as usize;
    if inner_width == 0 || viewport_lines == 0 {
        return 0;
    }

    let visual_lines = lines
        .iter()
        .map(|line| {
            let width = line.width();
            if width == 0 {
                1
            } else {
                width.div_ceil(inner_width)
            }
        })
        .sum::<usize>();

    visual_lines.saturating_sub(viewport_lines) as u16
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                " Terminal too small. Resize to at least {}×{} and try again.",
                MIN_WIDTH, MIN_HEIGHT
            ),
            Style::default().fg(theme::STATUS_WARN),
        )),
    ];
    let block = Block::default()
        .title(Span::styled(" WHETSTONE ", theme::header_title()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::AMBER));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn preview(s: &str, max: usize) -> String {
        s.chars().take(max).collect()
    }

    #[test]
    fn view_renders_dashboard_at_minimum_size() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let app = App::new(&tmp).unwrap();
        terminal.draw(|frame| view(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Sanity: the header string lands somewhere on the buffer.
        let rendered: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(rendered.contains("WHETSTONE"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn intro_screen_renders_full_frame() {
        let backend = TestBackend::new(100, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_intro_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.start_intro();
        terminal.draw(|frame| view(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(rendered.contains("Press enter to start"));
        assert!(rendered.contains("██╗    ██╗"));
        assert!(!rendered.contains("HOME"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn debt_screen_renders_not_computed_hint() {
        use crate::tui::app::DebtView;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_debt_empty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Debt;
        app.dashboard.debt = DebtView::NotComputed;
        terminal.draw(|frame| view(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect();
        assert!(
            rendered.contains("not computed"),
            "debt empty-state should show a hint; got: {}",
            preview(&rendered, 400)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn debt_screen_renders_error_state() {
        use crate::tui::app::DebtView;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_debt_err_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Debt;
        app.dashboard.debt = DebtView::Error("boom".into());
        terminal.draw(|frame| view(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_owned())
            .collect();
        assert!(rendered.contains("compute failed"));
        assert!(rendered.contains("boom"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn arrow_keys_move_selection_on_rules_screen() {
        use crate::tui::screens::rules::{RuleRow, RulesData, RulesView};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let tmp = std::env::temp_dir().join(format!("wh_tui_nav_rules_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;

        fn row(id: &str) -> RuleRow {
            RuleRow {
                id: id.into(),
                member_ids: vec![id.into()],
                severity: "should".into(),
                confidence: "high".into(),
                category: "convention".into(),
                languages: vec!["rust".into()],
                dep: id.split('.').next().unwrap_or(id).into(),
                layer: "project".into(),
                source_name: id.split('.').next().unwrap_or(id).into(),
                source_url: "https://example.com".into(),
                description: "x".into(),
                match_patterns: Vec::new(),
                lint_bindings: Vec::new(),
                formatter: None,
                tests: Vec::new(),
            }
        }
        app.dashboard.rules = RulesView::Ready(Box::new(RulesData {
            rows: vec![row("a.one"), row("b.two"), row("c.three")],
        }));
        app.rules_ui.language_filter = app::RulesLanguageFilter::Rust;

        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        if !matches!(&app.dashboard.rules, RulesView::Ready(_)) {
            panic!("rules view flipped out of Ready");
        }
        assert_eq!(
            app.rules_ui.selected, 2,
            "two Down presses should land on index 2"
        );

        // Down at the bottom clamps to the last row.
        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(app.rules_ui.selected, 2);

        // j/k work as vim aliases.
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.rules_ui.selected, 1);
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.rules_ui.selected, 0, "Up at the top clamps to 0");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn down_key_increments_dashboard_scroll() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let tmp =
            std::env::temp_dir().join(format!("wh_tui_dashboard_scroll_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Dashboard;

        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(app.dashboard_ui.scroll, 1);

        app.update(Msg::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.dashboard_ui.scroll, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn footer_renders_arrow_scroll_hints() {
        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_footer_scroll_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.dashboard_ui.scroll = 1;

        terminal.draw(|frame| view(frame, &app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(rendered.contains("↑ Up"));
        assert!(rendered.contains("↓ Down"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn footer_renders_form_submit_hints() {
        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_footer_form_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;
        app.input_mode = app::InputMode::RulesAdd;

        terminal.draw(|frame| view(frame, &app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(rendered.contains("ENTER: Submit"));
        assert!(rendered.contains("ESC: Cancel"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn view_renders_too_small_fallback() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp = std::env::temp_dir().join(format!("wh_tui_tiny_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let app = App::new(&tmp).unwrap();
        terminal.draw(|frame| view(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(rendered.contains("Terminal too small"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
