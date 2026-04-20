//! Whetstone evals for dependency: clap

use std::fs;
use std::path::Path;

fn find_rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !matches!(name.as_ref(), "target" | ".git" | "whetstone") {
                    files.extend(find_rust_files(&path));
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
}

// Rule: clap.derive-over-builder — SHOULD use the derive API (#[derive(Parser)]) instead of the builder API (Command::new()) for new CLI definitions. The derive API is more concise, type-safe, and the recommended approach in clap 4.x docs.

#[test]
fn test_clap_derive_over_builder_signal_0() {
    // Signal: Detects Command::new() or App::new() (legacy) in clap argument definitions (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"(Command|App)::new\s*\(").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for clap.derive-over-builder:\n{}", violations.len(), violations.join("\n"));
}

