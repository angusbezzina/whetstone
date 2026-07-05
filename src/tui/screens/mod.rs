//! Top-level screens. Each owns a `render(frame, area, app)` function and
//! the key hints it wants in the footer.

use ratatui::{layout::Rect, Frame};

use crate::tui::{app::App, components::footer, msg::Screen};

pub mod check;
pub mod dashboard;
pub mod debt;
pub mod extract;
pub mod help;
pub mod intro;
pub mod onboard;
pub mod result;
pub mod rules;
pub mod sources;

#[derive(Default, Clone)]
pub enum LoadState<T> {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<T>),
    Error(String),
}

impl<T> LoadState<T> {
    pub fn is_not_computed(&self) -> bool {
        matches!(self, Self::NotComputed)
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.screen {
        Screen::Intro => intro::render(frame, area, app),
        Screen::Dashboard => dashboard::render(frame, area, app),
        Screen::Result => result::render(frame, area, app),
        Screen::Sources => sources::render(frame, area, app),
        Screen::Rules => rules::render(frame, area, app),
        Screen::Check => check::render(frame, area, app),
        Screen::Debt => debt::render(frame, area, app),
        Screen::Help => help::render(frame, area, app),
        Screen::Onboard => onboard::render(frame, area, app),
    }
}

pub fn ensure_loaded(screen: Screen, app: &mut App) {
    match screen {
        Screen::Intro | Screen::Result | Screen::Dashboard | Screen::Help | Screen::Onboard => {}
        Screen::Debt => app.ensure_debt_loaded(),
        Screen::Sources => app.ensure_sources_loaded(),
        Screen::Rules => app.ensure_rules_loaded(),
        Screen::Check => app.ensure_check_loaded(),
    }
}

pub fn scroll_hint(screen: Screen, body: Rect, app: &App) -> Option<footer::ScrollHint> {
    match screen {
        Screen::Intro => None,
        Screen::Dashboard => hint_from_offset(
            app.dashboard_ui.scroll as u16,
            dashboard::max_scroll(body, app),
        ),
        Screen::Help => hint_from_offset(app.help.scroll_y, help::max_scroll(body)),
        Screen::Result => match &app.dashboard.result {
            crate::tui::screens::result::ResultView::Ready(data) => {
                hint_from_offset(data.scroll_y, result::max_scroll(body, data))
            }
            _ => None,
        },
        Screen::Sources => sources::scroll_hint(body, app),
        Screen::Rules => rules::scroll_hint(body, app),
        Screen::Debt => match &app.dashboard.debt {
            crate::tui::app::DebtView::Ready(data) => debt::scroll_hint(body, data),
            _ => None,
        },
        Screen::Check => None,
        Screen::Onboard => None,
    }
}

fn hint_from_offset(offset: u16, max_offset: u16) -> Option<footer::ScrollHint> {
    if max_offset == 0 {
        None
    } else {
        Some(footer::ScrollHint {
            up: offset > 0,
            down: offset < max_offset,
        })
    }
}
