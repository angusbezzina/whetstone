//! Whetstone TUI color palette.
//!
//! Amber `#FF7E00` is the brand accent for focus / selection / score fill.
//! Severity colors are semantic (must = red, should = yellow, may = cyan).
//! On 16-color terminals, [`amber`] falls back to [`Color::Yellow`].

use ratatui::style::{Color, Modifier, Style};

/// Brand accent — focused items, selected rows, gauge fill, active key labels.
pub const AMBER: Color = Color::Rgb(0xFF, 0x7E, 0x00);

/// Background shade for banded list rows on modern terminals.
#[allow(dead_code)]
pub const SLATE: Color = Color::Rgb(0x24, 0x29, 0x33);

/// Darker slate for inactive borders.
pub const MUTED: Color = Color::Rgb(0x6A, 0x73, 0x80);

// ── severity palette ──

pub const SEVERITY_MUST: Color = Color::Red;
pub const SEVERITY_SHOULD: Color = Color::Yellow;
pub const SEVERITY_MAY: Color = Color::Cyan;

// ── status palette ──

pub const STATUS_OK: Color = Color::Green;
pub const STATUS_WARN: Color = Color::Yellow;
#[allow(dead_code)]
pub const STATUS_ERR: Color = Color::Red;

// ── semantic styles ──

/// Bold amber for the accent key in key hints (e.g. `1`, `/`, `Q`).
pub fn key_hint_accent() -> Style {
    Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
}

/// Dim white for the descriptive label that follows the key (`HOME`, `FILTER`).
pub fn key_hint_label() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::DIM)
}

/// Bold amber for headers and screen titles.
pub fn header_title() -> Style {
    Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
}

/// Dim white for secondary header text (breadcrumb, version).
pub fn header_meta() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::DIM)
}

/// Active border — used for the focused pane.
pub fn border_active() -> Style {
    Style::default().fg(AMBER)
}

/// Inactive border — muted slate for non-focused panes.
pub fn border_inactive() -> Style {
    Style::default().fg(MUTED)
}

/// Highlight row style for a selected list item.
#[allow(dead_code)]
pub fn selection() -> Style {
    Style::default()
        .bg(SLATE)
        .fg(AMBER)
        .add_modifier(Modifier::BOLD)
}

/// Color for a severity badge.
pub fn severity_color(severity: &str) -> Color {
    match severity {
        "must" | "MUST" => SEVERITY_MUST,
        "should" | "SHOULD" => SEVERITY_SHOULD,
        "may" | "MAY" => SEVERITY_MAY,
        _ => MUTED,
    }
}
