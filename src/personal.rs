//! Personal layer scaffolding — `whetstone/.personal/` directory, gitignore,
//! and helpers that the CLI calls when the user runs `wh init --personal` or
//! `wh promote`.
//!
//! Personal rules stay LOCAL. They never appear in committed outputs and
//! `.personal/` is auto-added to `.gitignore` on first setup so a teammate
//! cannot accidentally commit another teammate's overrides.

use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const PERSONAL_SUBDIRS: &[&str] = &["rules", "evals", "lint", "context"];

const GITIGNORE_MARKER: &str = "# Whetstone personal layer (do not commit)";
const GITIGNORE_RULES: &[&str] = &[
    "whetstone/.personal/",
    "whetstone/.cache/",
    "whetstone/.state/",
    "whetstone/.metrics.jsonl",
    "whetstone/.last-run",
];

const DEFAULT_PERSONAL_CONFIG: &str = r#"# whetstone/.personal/config.yaml
#
# Personal overrides for the current user. This file is gitignored — nothing
# here is shipped alongside the project. Use it to deny rules you disagree
# with or to tag personal sources.

deny: []
"#;

/// Initialize `whetstone/.personal/` on disk: create subdirectories, write a
/// default config file, and ensure `.gitignore` hides the layer.
pub fn init_personal(project_dir: &Path) -> Result<Value> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut created: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for sub in PERSONAL_SUBDIRS {
        let path = paths.personal_dir.join(sub);
        if path.exists() {
            skipped.push(path.display().to_string());
        } else {
            fs::create_dir_all(&path)?;
            created.push(path.display().to_string());
        }
    }

    if !paths.personal_config.exists() {
        fs::write(&paths.personal_config, DEFAULT_PERSONAL_CONFIG)?;
        created.push(paths.personal_config.display().to_string());
    } else {
        skipped.push(paths.personal_config.display().to_string());
    }

    let gitignore_status = ensure_gitignore_entries(project_dir)?;

    Ok(json!({
        "status": "ok",
        "personal_dir": paths.personal_dir.display().to_string(),
        "created": created,
        "skipped_existing": skipped,
        "gitignore": gitignore_status,
        "next_command": "Drop rule YAML files in whetstone/.personal/rules/<language>/. Run `wh context --personal` / `wh tests --personal` to generate outputs.",
    }))
}

fn ensure_gitignore_entries(project_dir: &Path) -> Result<Value> {
    let gi_path = project_dir.join(".gitignore");
    let existing = fs::read_to_string(&gi_path).unwrap_or_default();

    let mut already_has: Vec<String> = Vec::new();
    let mut to_add: Vec<String> = Vec::new();

    for rule in GITIGNORE_RULES {
        if gitignore_contains(&existing, rule) {
            already_has.push((*rule).to_string());
        } else {
            to_add.push((*rule).to_string());
        }
    }

    if to_add.is_empty() {
        return Ok(json!({
            "path": gi_path.display().to_string(),
            "action": "noop",
            "already_present": already_has,
        }));
    }

    let mut new_content = existing.clone();
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    if !new_content.contains(GITIGNORE_MARKER) {
        new_content.push('\n');
        new_content.push_str(GITIGNORE_MARKER);
        new_content.push('\n');
    }
    for rule in &to_add {
        new_content.push_str(rule);
        new_content.push('\n');
    }
    fs::write(&gi_path, new_content)?;

    Ok(json!({
        "path": gi_path.display().to_string(),
        "action": "updated",
        "added": to_add,
        "already_present": already_has,
    }))
}

fn gitignore_contains(content: &str, rule: &str) -> bool {
    content.lines().any(|line| line.trim() == rule)
}

