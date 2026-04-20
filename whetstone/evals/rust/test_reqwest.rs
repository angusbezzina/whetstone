//! Whetstone evals for dependency: reqwest

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

// Rule: reqwest.set-timeout — MUST set an explicit timeout on reqwest clients. The default is no timeout, which means requests can hang indefinitely on unresponsive servers.

#[test]
fn test_reqwest_set_timeout_signal_0() {
    // Signal: Detects Client::new() or ClientBuilder::new()...build() without .timeout() (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"Client::new\s*\(\)").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for reqwest.set-timeout:\n{}", violations.len(), violations.join("\n"));
}

// Rule: reqwest.check-status — SHOULD call .error_for_status() or explicitly check status codes on reqwest responses. By default, reqwest treats 4xx/5xx as successful responses and returns the body without error.

#[test]
fn test_reqwest_check_status_signal_0() {
    // Signal: Detects .get()/.post() chains without .error_for_status() or .status() check (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"\.send\s*\(\)\s*\?\s*\.\s*text\s*\(").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for reqwest.check-status:\n{}", violations.len(), violations.join("\n"));
}

