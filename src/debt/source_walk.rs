//! Walk the project for source files the debt detectors care about.
//! Reuses `detect::walk::SKIP_DIRS` so the scan matches other commands.

use std::path::{Path, PathBuf};

use crate::detect::walk::SKIP_DIRS;

use super::types::SourceInventory;

const PY_EXTS: &[&str] = &["py"];
const TS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];
const RS_EXTS: &[&str] = &["rs"];

/// Collect all source files under `project_dir`, bucketed by language.
/// Ignores test-fixture dirs, vendored code, build outputs, and
/// whetstone's own working directory.
pub fn collect(project_dir: &Path) -> SourceInventory {
    let skip: std::collections::HashSet<&str> = SKIP_DIRS
        .iter()
        .filter(|s| !s.contains('/'))
        .copied()
        .collect();
    let skip_multi: Vec<&str> = SKIP_DIRS
        .iter()
        .filter(|s| s.contains('/'))
        .copied()
        .collect();

    let mut out = SourceInventory::default();
    walk_dir(project_dir, project_dir, &skip, &skip_multi, &mut out);
    out.python.sort();
    out.typescript.sort();
    out.rust.sort();
    out
}

fn walk_dir(
    dir: &Path,
    project_dir: &Path,
    skip: &std::collections::HashSet<&str>,
    skip_multi: &[&str],
    out: &mut SourceInventory,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if name.starts_with('.') && name != ".whetstone" {
                continue;
            }
            if skip.contains(name) {
                continue;
            }
            let rel = path
                .strip_prefix(project_dir)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let is_multi_skip = skip_multi
                .iter()
                .any(|m| rel == *m || rel.starts_with(&format!("{m}/")));
            if is_multi_skip {
                continue;
            }
            walk_dir(&path, project_dir, skip, skip_multi, out);
        } else if path.is_file() {
            classify(&path, out);
        }
    }
}

fn classify(path: &Path, out: &mut SourceInventory) {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return,
    };
    let pb: PathBuf = path.to_path_buf();
    if PY_EXTS.contains(&ext) {
        out.python.push(pb);
    } else if TS_EXTS.contains(&ext) {
        out.typescript.push(pb);
    } else if RS_EXTS.contains(&ext) {
        out.rust.push(pb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn classifies_by_extension() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/a.py"), "x = 1\n").unwrap();
        fs::write(root.join("pkg/b.ts"), "export const x = 1;\n").unwrap();
        fs::write(root.join("pkg/c.rs"), "fn x() {}\n").unwrap();
        fs::write(root.join("README.md"), "not source\n").unwrap();

        let inv = collect(root);
        assert_eq!(inv.python.len(), 1);
        assert_eq!(inv.typescript.len(), 1);
        assert_eq!(inv.rust.len(), 1);
    }

    #[test]
    fn skips_vendor_and_build_dirs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("node_modules/a.ts"), "ignored\n").unwrap();
        fs::write(root.join("target/a.rs"), "ignored\n").unwrap();
        fs::write(root.join("src/a.rs"), "fn x() {}\n").unwrap();

        let inv = collect(root);
        assert_eq!(inv.rust.len(), 1, "only src/a.rs should be included");
        assert_eq!(inv.typescript.len(), 0);
    }
}
