use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Directories to skip when searching for manifests.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".mypy_cache",
    ".ruff_cache",
    ".pytest_cache",
    ".tox",
    ".nox",
    ".venv",
    "venv",
    "env",
    ".env",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".turbo",
    ".vercel",
    ".output",
    "coverage",
    ".whetstone",
    "whetstone",
    "tests/fixtures",
    "fixtures",
    "examples",
    "vendor",
    "third_party",
    "third-party",
];

const MANIFEST_NAMES: &[&str] = &[
    "pyproject.toml",
    "requirements.txt",
    "package.json",
    "Cargo.toml",
];

/// Recursively find all manifest files under project_dir.
/// Returns (absolute_path, source_dir) tuples.
pub fn find_manifests(
    project_dir: &Path,
    extra_excludes: &[String],
    extra_includes: &[String],
) -> Vec<(PathBuf, String)> {
    let manifest_set: HashSet<&str> = MANIFEST_NAMES.iter().copied().collect();
    let mut results: Vec<(PathBuf, String)> = Vec::new();

    // Split skip dirs into single-segment and multi-segment
    let single_seg: HashSet<&str> = SKIP_DIRS
        .iter()
        .filter(|s| !s.contains('/'))
        .copied()
        .collect();
    let multi_seg: Vec<&str> = SKIP_DIRS.iter().filter(|s| s.contains('/')).copied().collect();

    let extra_single: HashSet<String> = extra_excludes
        .iter()
        .filter(|s| !s.contains('/'))
        .cloned()
        .collect();
    let extra_multi: Vec<&str> = extra_excludes
        .iter()
        .filter(|s| s.contains('/'))
        .map(|s| s.as_str())
        .collect();

    walk_dir(
        project_dir,
        project_dir,
        &manifest_set,
        &single_seg,
        &multi_seg,
        &extra_single,
        &extra_multi,
        extra_includes,
        &mut results,
    );

    results
}

#[allow(clippy::too_many_arguments)]
fn walk_dir(
    dir: &Path,
    project_dir: &Path,
    manifest_set: &HashSet<&str>,
    single_seg: &HashSet<&str>,
    multi_seg: &[&str],
    extra_single: &HashSet<String>,
    extra_multi: &[&str],
    includes: &[String],
    results: &mut Vec<(PathBuf, String)>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.is_file() {
            files.push(path);
        }
    }

    // Check for manifest files
    let rel_dir = dir
        .strip_prefix(project_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let source = if rel_dir.is_empty() || rel_dir == "." {
        "root".to_string()
    } else {
        rel_dir.clone()
    };

    for file in &files {
        if let Some(name) = file.file_name().and_then(|n| n.to_str()) {
            if manifest_set.contains(name) {
                results.push((file.clone(), source.clone()));
            }
        }
    }

    // Recurse into subdirectories
    dirs.sort();
    for subdir in &dirs {
        let dir_name = subdir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Skip hidden dirs (except specific ones)
        if dir_name.starts_with('.') && dir_name != ".whetstone" && dir_name != ".env" {
            continue;
        }

        let child_rel = if rel_dir.is_empty() || rel_dir == "." {
            dir_name.to_string()
        } else {
            format!("{rel_dir}/{dir_name}")
        };

        if should_skip_dir(
            dir_name,
            &child_rel,
            single_seg,
            multi_seg,
            extra_single,
            extra_multi,
            includes,
        ) {
            continue;
        }

        walk_dir(
            subdir,
            project_dir,
            manifest_set,
            single_seg,
            multi_seg,
            extra_single,
            extra_multi,
            includes,
            results,
        );
    }
}

fn should_skip_dir(
    dir_name: &str,
    rel_path: &str,
    single_seg: &HashSet<&str>,
    multi_seg: &[&str],
    extra_single: &HashSet<String>,
    extra_multi: &[&str],
    includes: &[String],
) -> bool {
    // Include overrides exclude
    for inc in includes {
        if rel_path == inc || rel_path.starts_with(&format!("{inc}/")) || dir_name == inc {
            return false;
        }
    }

    // Single-segment checks
    if single_seg.contains(dir_name) || extra_single.contains(dir_name) {
        return true;
    }

    // Multi-segment checks
    for excl in multi_seg.iter().chain(extra_multi.iter()) {
        if rel_path == *excl || rel_path.starts_with(&format!("{excl}/")) {
            return true;
        }
    }

    false
}
