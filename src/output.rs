use std::io::IsTerminal;

use serde_json::Value;

/// Check whether stdout is a pipe (not an interactive terminal).
/// When piped, commands auto-switch to JSON output so scripts and agents
/// can consume the output without passing `--json`.
pub fn is_piped() -> bool {
    !std::io::stdout().is_terminal()
}

/// Print JSON to stdout with a trailing newline (matching Python contract).
pub fn print_json(value: &Value) {
    if let Ok(s) = serde_json::to_string_pretty(value) {
        println!("{s}");
    }
}

/// Print a progress message to stderr (so stdout stays clean for JSON).
pub fn log(msg: &str) {
    eprintln!("{msg}");
}

/// Build a JSON error response matching the Python contract.
pub fn error_json(error: &str, next_command: &str) -> Value {
    serde_json::json!({
        "error": error,
        "next_command": next_command,
    })
}

/// Box-drawing report builder matching the Python doctor/status report format.
pub struct ReportBuilder {
    lines: Vec<String>,
    width: usize,
}

impl ReportBuilder {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            width: 62,
        }
    }

    pub fn top_border(&mut self) {
        self.lines
            .push(format!("\u{2554}{}\u{2557}", "\u{2550}".repeat(self.width)));
    }

    pub fn bottom_border(&mut self) {
        self.lines
            .push(format!("\u{255a}{}\u{255d}", "\u{2550}".repeat(self.width)));
    }

    pub fn line(&mut self, text: &str) {
        let pad = self.width.saturating_sub(2 + text.len());
        self.lines
            .push(format!("\u{2551}  {}{}\u{2551}", text, " ".repeat(pad)));
    }

    pub fn empty_line(&mut self) {
        self.line("");
    }

    pub fn section_header(&mut self, title: &str) {
        let padded = format!("\u{2550}\u{2550} {} ", title);
        let fill = self.width.saturating_sub(padded.len());
        self.lines.push(format!(
            "\u{2560}{}{}\u{2550}\u{2563}",
            padded,
            "\u{2550}".repeat(fill.saturating_sub(1))
        ));
    }

    pub fn build(&self) -> String {
        self.lines.join("\n")
    }
}
