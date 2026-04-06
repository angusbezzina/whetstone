//! Whetstone eval: serde_yaml.crate-deprecated
//! serde_yaml 0.9 is officially deprecated and unmaintained. MUST migrate to an actively maintained alternative such as serde_yml, yaml-rust2, or marked_yaml.

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
fn test_serde_yaml_crate_deprecated_signal_0() {
    // Signal: Cargo.toml or source files reference serde_yaml (pattern)
    let files = find_rust_files(Path::new("src"));
    let mut violations = Vec::new();
    for file in &files {
        if let Ok(content) = fs::read_to_string(file) {
            // TODO: implement check for: Cargo.toml or source files reference serde_yaml
            let _ = content;
        }
    }
    assert!(violations.is_empty(), "{} violations for serde_yaml.crate-deprecated", violations.len());
}
