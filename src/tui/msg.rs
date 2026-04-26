//! Top-level message types for the Elm-style update loop.
//!
//! Every keystroke + timer tick enters `update` as a `Msg`. No side effects
//! happen here — only state transitions. Async work (e.g. fetching a source)
//! posts follow-up `Msg`s when it completes.

use crossterm::event::KeyEvent;

/// Identifies which top-level screen is active. Navigation via `1`–`7` or
/// within-screen actions producing `Msg::GoToScreen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Result,
    Rules,
    Sources,
    Extract,
    Check,
    Report,
    Drift,
    Debt,
    Help,
}

impl Screen {
    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "DASHBOARD",
            Screen::Result => "RESULT",
            Screen::Rules => "RULES",
            Screen::Sources => "SOURCES",
            Screen::Extract => "EXTRACT",
            Screen::Check => "CHECK",
            Screen::Report => "REPORT",
            Screen::Drift => "DRIFT",
            Screen::Debt => "DEBT",
            Screen::Help => "HELP",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // GoToScreen/Refresh/Tick/Quit are emitted by future async sources
pub enum Msg {
    /// Raw key event — the update function decodes it into higher-level messages.
    Key(KeyEvent),
    /// Jump to a specific top-level screen.
    GoToScreen(Screen),
    /// Re-load dashboard / per-screen data from disk + `wh check`.
    Refresh,
    /// Periodic tick (every ~250ms) for spinner animation.
    Tick,
    /// User pressed Q / Ctrl-C. Exit the loop cleanly.
    Quit,
}
