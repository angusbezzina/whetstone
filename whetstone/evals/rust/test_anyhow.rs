//! Whetstone eval: anyhow.expect-over-unwrap
//! SHOULD use .expect("reason") instead of .unwrap() in application code using anyhow. When an operation is expected to succeed, .expect() documents the invariant and produces actionable panic messages.

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

#[test]
fn test_anyhow_expect_over_unwrap_signal_0() {
    // Signal: Detects .unwrap() calls in source files (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            // TODO: implement check for: Detects .unwrap() calls in source files
            let _ = content;
        }
    }
    assert!(violations.is_empty(), "{} violations for anyhow.expect-over-unwrap", violations.len());
}
