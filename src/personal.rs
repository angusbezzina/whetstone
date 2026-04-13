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
    let personal_dir = project_dir.join("whetstone").join(".personal");
    let mut created: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for sub in PERSONAL_SUBDIRS {
        let path = personal_dir.join(sub);
        if path.exists() {
            skipped.push(path.display().to_string());
        } else {
            fs::create_dir_all(&path)?;
            created.push(path.display().to_string());
        }
    }

    let config_path = personal_dir.join("config.yaml");
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_PERSONAL_CONFIG)?;
        created.push(config_path.display().to_string());
    } else {
        skipped.push(config_path.display().to_string());
    }

    let gitignore_status = ensure_gitignore_entries(project_dir)?;

    Ok(json!({
        "status": "ok",
        "personal_dir": personal_dir.display().to_string(),
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
    let target_layer = match target {
        "project" | "team" | "personal" => target,
        other => {
            return Err(anyhow!(
                "Unknown --to layer '{other}'. Expected: personal, project, team."
            ))
        }
    };

    let whetstone_dir = project_dir.join("whetstone");
    let personal_rules = whetstone_dir.join(".personal").join("rules");
    let project_rules = whetstone_dir.join("rules");
    let team_rules = whetstone_dir.join(".team").join("rules");

    let source = find_rule_source(&personal_rules, &project_rules, rule_id)?;

    // Enforce monotonic promotion (never go "down").
    let direction = (source.layer, target_layer);
    let allowed = matches!(
        direction,
        ("personal", "project")
            | ("personal", "team")
            | ("project", "team")
    );
    if !allowed {
        return Err(anyhow!(
            "Cannot promote from {} to {target_layer}. Promotion is monotonic (personal → project → team).",
            source.layer
        ));
    }

    let dest_dir = match target_layer {
        "project" => project_rules,
        "team" => team_rules,
        "personal" => personal_rules,
        _ => unreachable!(),
    };

    let dest_filename = source
        .file
        .file_name()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("Source rule file has no filename"))?;

    // Preserve language subdirectory if the source was placed under one.
    let lang_subdir = source
        .file
        .parent()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    let dest_parent = if matches!(lang_subdir.as_str(), "python" | "typescript" | "rust") {
        dest_dir.join(&lang_subdir)
    } else {
        dest_dir
    };
    fs::create_dir_all(&dest_parent)?;
    let dest_path = dest_parent.join(&dest_filename);

    if dest_path.exists() {
        return Err(anyhow!(
            "Destination already exists: {}. Delete it first, or pick a different target.",
            dest_path.display()
        ));
    }

    // Perform the move/copy of the full rule file. We preserve the whole YAML
    // so siblings travel with the promoted rule — if the user wanted to split,
    // they would have done so already. Document this in the returned status.
    let body = fs::read_to_string(&source.file)?;
    fs::write(&dest_path, body)?;

    if !keep_source {
        fs::remove_file(&source.file)?;
    }

    Ok(json!({
        "status": "ok",
        "rule_id": rule_id,
        "from": source.layer,
        "to": target_layer,
        "source_file": source.file.display().to_string(),
        "destination_file": dest_path.display().to_string(),
        "kept_source": keep_source,
        "note": "Whole rule file was moved; siblings in the same file travel together.",
        "next_command": "wh validate && wh context && wh tests",
    }))
}

struct SourceRule {
    file: PathBuf,
    layer: &'static str,
}

fn find_rule_source(
    personal_rules: &Path,
    project_rules: &Path,
    rule_id: &str,
) -> Result<SourceRule> {
    if let Some(p) = crate::layers::find_rule_file(personal_rules, rule_id) {
        return Ok(SourceRule {
            file: p,
            layer: "personal",
        });
    }
    if let Some(p) = crate::layers::find_rule_file(project_rules, rule_id) {
        return Ok(SourceRule {
            file: p,
            layer: "project",
        });
    }
    Err(anyhow!(
        "Rule '{rule_id}' not found in whetstone/.personal/rules/ or whetstone/rules/."
    ))
}
