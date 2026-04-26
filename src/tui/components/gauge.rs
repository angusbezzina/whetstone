//! Score gauge — amber-filled block bar 0–100.
//!
//! Renders as e.g. `▰▰▰▰▰▰▰▱▱▱  72` at the specified width. Null / unavailable
//! scores render as plain `N/A` (no bar).

use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
};

use crate::tui::theme;

const FILLED: char = '▰';
const EMPTY: char = '▱';

pub fn render(score: Option<i64>, width: usize) -> Line<'static> {
    let bar_width = width.max(6);
    match score {
        Some(s) => {
            let pct = s.clamp(0, 100) as f64 / 100.0;
            let filled = ((bar_width as f64) * pct).round() as usize;
            let empty = bar_width.saturating_sub(filled);
            let bar: String =
                std::iter::repeat(FILLED).take(filled).collect::<String>()
                    + &std::iter::repeat(EMPTY).take(empty).collect::<String>();
            Line::from(vec![
                Span::styled(bar, Style::default().fg(theme::AMBER)),
                Span::raw(format!(" {:>3}", s)),
            ])
        }
        None => Line::from(Span::styled("N/A", Style::default().fg(theme::MUTED).bold())),
    }
}
