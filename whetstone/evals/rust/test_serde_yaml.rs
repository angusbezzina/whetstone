//! Whetstone evals for dependency: serde_yaml

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

// Rule: serde_yaml.crate-deprecated — serde_yaml 0.9 is officially deprecated and unmaintained. MUST migrate to an actively maintained alternative such as serde_yml, yaml-rust2, or marked_yaml.

#[test]
fn test_serde_yaml_crate_deprecated_signal_0() {
    // Signal: Cargo.toml or source files reference serde_yaml (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    let pattern = regex::Regex::new(r"serde_yaml").unwrap();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            for (line_num, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{} violations for serde_yaml.crate-deprecated:\n{}", violations.len(), violations.join("\n"));
}

