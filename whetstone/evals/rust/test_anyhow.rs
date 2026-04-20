//! Whetstone evals for dependency: anyhow

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

// Rule: anyhow.context-over-map-err — SHOULD use .context() or .with_context() instead of .map_err(|e| anyhow!(...)) to add context to errors. The context methods are more idiomatic, compose better with the ? operator, and preserve the original error chain.

#[test]
fn test_anyhow_context_over_map_err_signal_0() {
    // Signal: Detects .map_err followed by anyhow! macro call (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"\.map_err\s*\(.*anyhow!").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for anyhow.context-over-map-err:\n{}", violations.len(), violations.join("\n"));
}

// Rule: anyhow.expect-over-unwrap — SHOULD use .expect("reason") instead of .unwrap() in application code using anyhow. When an operation is expected to succeed, .expect() documents the invariant and produces actionable panic messages.

#[test]
fn test_anyhow_expect_over_unwrap_signal_0() {
    // Signal: Detects .unwrap() calls in source files (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"\.unwrap\s*\(\)").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for anyhow.expect-over-unwrap:\n{}", violations.len(), violations.join("\n"));
}

