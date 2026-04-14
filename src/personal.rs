//! Personal layer scaffolding — `whetstone/.personal/` directory, gitignore,
//! and helpers that the CLI calls when the user runs `wh init --personal` or
//! `wh promote`.
//!
//! Personal rules stay LOCAL. They never appear in committed outputs and
//! `.personal/` is auto-added to `.gitignore` on first setup so a teammate
//! cannot accidentally commit another teammate's overrides.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::layers::Layer;

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

/// Promote a rule from one layer to another. The CLI exposes this as
/// `wh promote <rule-id> --to <layer>`. Returns a structured summary.
///
/// Supported directions:
/// - personal → project  (share a personal rule with the team)
/// - project → team (publisher workflow: write to `whetstone/.team/`)
/// - personal → team
///
/// Copying "down" (e.g. built-in → project) is a manual override, not a
/// promotion, and intentionally not supported here.
pub fn promote_rule(
    project_dir: &Path,
    rule_id: &str,
    target: &str,
    keep_source: bool,
) -> Result<Value> {
    let target_layer = parse_promote_target(target)?;

    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let source = find_rule_source(&paths, rule_id)?;

    if !is_monotonic_promotion(source.layer, target_layer) {
        return Err(anyhow!(
            "Cannot promote from {} to {}. Promotion is monotonic (personal → project → team).",
            source.layer.as_str(),
            target_layer.as_str(),
        ));
    }

    let dest_dir = match target_layer {
        Layer::Personal => &paths.personal_rules_dir,
        Layer::Project => &paths.project_rules_dir,
        Layer::Team => &paths.team_staging_rules_dir,
        Layer::BuiltIn => {
            return Err(anyhow!(
                "Cannot promote into the built-in layer — built-ins ship inside the binary."
            ));
        }
    };

    let dest_filename = source
        .file
        .file_name()
        .ok_or_else(|| anyhow!("Source rule file has no filename"))?;

    let lang_subdir = source
        .file
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dest_parent = if matches!(lang_subdir.as_str(), "python" | "typescript" | "rust") {
        dest_dir.join(&lang_subdir)
    } else {
        dest_dir.clone()
    };
    fs::create_dir_all(&dest_parent)?;
    let dest_path = dest_parent.join(dest_filename);

    // Atomic create-new: open-or-fail, no TOCTOU race with a concurrent writer.
    let body = fs::read_to_string(&source.file)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest_path)
    {
        Ok(mut f) => {
            use std::io::Write as _;
            f.write_all(body.as_bytes())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(anyhow!(
                "Destination already exists: {}. Delete it first, or pick a different target.",
                dest_path.display()
            ));
        }
        Err(e) => return Err(e.into()),
    }

    if !keep_source {
        fs::remove_file(&source.file)?;
    }

    Ok(json!({
        "status": "ok",
        "rule_id": rule_id,
        "from": source.layer.as_str(),
        "to": target_layer.as_str(),
        "source_file": source.file.display().to_string(),
        "destination_file": dest_path.display().to_string(),
        "kept_source": keep_source,
        "note": "Whole rule file was moved; siblings in the same file travel together.",
        "next_command": "wh validate && wh context && wh tests",
    }))
}

fn parse_promote_target(raw: &str) -> Result<Layer> {
    match raw {
        "personal" => Ok(Layer::Personal),
        "project" => Ok(Layer::Project),
        "team" => Ok(Layer::Team),
        other => Err(anyhow!(
            "Unknown --to layer '{other}'. Expected: personal, project, team."
        )),
    }
}

fn is_monotonic_promotion(from: Layer, to: Layer) -> bool {
    use Layer::*;
    matches!(
        (from, to),
        (Personal, Project) | (Personal, Team) | (Project, Team)
    )
}

struct SourceRule {
    file: PathBuf,
    layer: Layer,
}

fn find_rule_source(paths: &crate::layers::LayerPaths, rule_id: &str) -> Result<SourceRule> {
    if let Some(p) = crate::layers::find_rule_file(&paths.personal_rules_dir, rule_id) {
        return Ok(SourceRule {
            file: p,
            layer: Layer::Personal,
        });
    }
    if let Some(p) = crate::layers::find_rule_file(&paths.project_rules_dir, rule_id) {
        return Ok(SourceRule {
            file: p,
            layer: Layer::Project,
        });
    }
    Err(anyhow!(
        "Rule '{rule_id}' not found in whetstone/.personal/rules/ or whetstone/rules/."
    ))
}
