//! Top-level message types for the Elm-style update loop.
//!
//! Every keystroke enters `update` as a `Msg`. No side effects happen here —
//! only state transitions.

use crossterm::event::KeyEvent;

/// Identifies which top-level screen is active. Navigation uses `1`–`5`
/// through the centralized screen registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Intro,
    Dashboard,
    Result,
    Sources,
    Rules,
    Check,
    Debt,
    Help,
}

impl Screen {
    const NAV_HINTS: [(&'static str, &'static str); 5] = [
        ("1", "HOME"),
        ("2", "SOURCES"),
        ("3", "RULES"),
        ("4", "VIOLATIONS"),
        ("5", "DEBT"),
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Intro => "INTRO",
            Screen::Dashboard => "HOME",
            Screen::Result => "RESULT",
            Screen::Sources => "SOURCES",
            Screen::Rules => "RULES",
            Screen::Check => "VIOLATIONS",
            Screen::Debt => "DEBT",
            Screen::Help => "HELP",
        }
    }

    pub fn from_nav_key(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Dashboard),
            '2' => Some(Self::Sources),
            '3' => Some(Self::Rules),
            '4' => Some(Self::Check),
            '5' => Some(Self::Debt),
            _ => None,
        }
    }

    pub fn nav_hints() -> &'static [(&'static str, &'static str)] {
        &Self::NAV_HINTS
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Raw key event — the update function decodes it into higher-level messages.
    Key(KeyEvent),
    /// Jump to a specific top-level screen.
    GoToScreen(Screen),
}
