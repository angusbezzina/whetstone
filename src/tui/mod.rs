//! Whetstone TUI — interactive dashboard for Epic 4A.
//!
//! Invoked by `wh tui` OR by a bare `wh` on a TTY. Elm-style loop:
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

/// Check whether stdout is a TTY. `wh` with no args uses this to decide
/// whether to launch the TUI or dump the CLI help.
pub fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    io::stdout().is_terminal()
}

/// Blocking entry point. Sets up the terminal, runs the main loop, restores.
pub fn run(project_dir: &Path) -> Result<()> {
    let mut app = App::new(project_dir).context("failed to load project data")?;

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
            app.dashboard.result = screens::result::ResultView::Ready(Box::new(
                screens::result::ResultData { title, body },
            ));
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

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
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

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| view(frame, app))?;

        // Tight poll interval keeps input latency low while letting us do
        // future background work (drift polling, spinner ticks) cheaply.
        if event::poll(Duration::from_millis(100))? {
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
    let hints: &[footer::Hint] = match app.screen {
        Screen::Dashboard => screens::dashboard::hints(),
        Screen::Result => screens::result::hints(),
        Screen::Rules => screens::rules::hints(),
        Screen::Sources => screens::sources::hints(),
        Screen::Extract => screens::extract::hints(),
        Screen::Check => screens::check::hints(),
        Screen::Report => screens::report::hints(),
        Screen::Drift => screens::drift::hints(),
        Screen::Debt => screens::debt::hints(),
        Screen::Help => screens::help::hints(),
    };

    match app.screen {
        Screen::Dashboard => screens::dashboard::render(frame, body, app),
        Screen::Result => screens::result::render(frame, body, app),
        Screen::Rules => screens::rules::render(frame, body, app),
        Screen::Sources => screens::sources::render(frame, body, app),
        Screen::Extract => screens::extract::render(frame, body, app),
        Screen::Check => screens::check::render(frame, body, app),
        Screen::Report => screens::report::render(frame, body, app),
        Screen::Drift => screens::drift::render(frame, body, app),
        Screen::Debt => screens::debt::render(frame, body, app),
        Screen::Help => screens::help::render(frame, body),
    }

    footer::render(frame, chunks[2], hints);
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
        assert!(rendered.contains("Whetstone"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn debt_screen_renders_not_computed_hint() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let tmp =
            std::env::temp_dir().join(format!("wh_tui_debt_empty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Debt; // without ensure_debt_loaded — stays NotComputed
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
            &rendered[..rendered.len().min(400)]
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
        use crate::tui::screens::rules::{RulesData, RulesView, RuleRow};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let tmp =
            std::env::temp_dir().join(format!("wh_tui_nav_rules_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;

        fn row(id: &str) -> RuleRow {
            RuleRow {
                id: id.into(),
                severity: "should".into(),
                confidence: "high".into(),
                language: "rust".into(),
                dep: id.split('.').next().unwrap_or(id).into(),
                layer: "project".into(),
                source_url: "https://example.com".into(),
                description: "x".into(),
            }
        }
        app.dashboard.rules = RulesView::Ready(Box::new(RulesData {
            rows: vec![row("a.one"), row("b.two"), row("c.three")],
            by_language: vec![("rust".into(), 3)],
            selected: 0,
        }));

        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        if let RulesView::Ready(d) = &app.dashboard.rules {
            assert_eq!(d.selected, 2, "two Down presses should land on index 2");
        } else {
            panic!("rules view flipped out of Ready");
        }

        // Down at the bottom clamps to the last row.
        app.update(Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        if let RulesView::Ready(d) = &app.dashboard.rules {
            assert_eq!(d.selected, 2);
        }

        // j/k work as vim aliases.
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        if let RulesView::Ready(d) = &app.dashboard.rules {
            assert_eq!(d.selected, 1);
        }
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        if let RulesView::Ready(d) = &app.dashboard.rules {
            assert_eq!(d.selected, 0, "Up at the top clamps to 0");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pagedown_advances_report_scroll() {
        use crate::tui::screens::report::{ReportData, ReportView};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let tmp =
            std::env::temp_dir().join(format!("wh_tui_nav_report_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Report;
        app.dashboard.report = ReportView::Ready(Box::new(ReportData {
            markdown: "line\n".repeat(100),
            scroll: 0,
        }));

        app.update(Msg::Key(KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        if let ReportView::Ready(d) = &app.dashboard.report {
            assert_eq!(d.scroll, 10);
        } else {
            panic!("report flipped out of Ready");
        }

        app.update(Msg::Key(KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));
        if let ReportView::Ready(d) = &app.dashboard.report {
            assert_eq!(d.scroll, 0);
        }

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
