use std::io::IsTerminal;

use serde_json::Value;

pub fn is_piped() -> bool {
    !std::io::stdout().is_terminal()
}

pub fn print_json(value: &Value) {
    if let Ok(serialized) = serde_json::to_string_pretty(value) {
        println!("{serialized}");
    }
}
