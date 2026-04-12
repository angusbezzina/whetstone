//! Whetstone eval: rust.must-use-results
//! SHOULD not discard Result values. Every Result should be handled via ?, .unwrap(), .expect(), match, or explicit let _ = assignment.

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
fn test_rust_must_use_results_signal_0() {
    // Signal: Function call returning Result with no binding or ? operator (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            // TODO: implement check for: Function call returning Result with no binding or ? operator
            let _ = content;
        }
    }
    assert!(violations.is_empty(), "{} violations for rust.must-use-results:\n{}", violations.len(), violations.join("\n"));
}
