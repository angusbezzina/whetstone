//! Top-level message types for the Elm-style update loop.
//!
//! Every keystroke + timer tick enters `update` as a `Msg`. No side effects
//! happen here — only state transitions. Async work (e.g. fetching a source)
//! posts follow-up `Msg`s when it completes.

use crossterm::event::KeyEvent;

/// Identifies which top-level screen is active. Navigation via `1`–`6` or
/// within-screen actions producing `Msg::GoToScreen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Result,
    Extract,
    Sources,
    Rules,
    Check,
    Debt,
    Help,
}

impl Screen {
    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "HOME",
            Screen::Result => "RESULT",
            Screen::Extract => "INTERNAL SOURCES",
            Screen::Sources => "EXTERNAL SOURCES",
            Screen::Rules => "RULES",
            Screen::Check => "VIOLATIONS",
            Screen::Debt => "DEBT",
            Screen::Help => "HELP",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // GoToScreen/Tick/Quit are emitted by future async sources
pub enum Msg {
    /// Raw key event — the update function decodes it into higher-level messages.
    Key(KeyEvent),
    /// Jump to a specific top-level screen.
    GoToScreen(Screen),
    /// Periodic tick (every ~250ms) for spinner animation.
    Tick,
    /// User pressed Q / Ctrl-C. Exit the loop cleanly.
    Quit,
}
