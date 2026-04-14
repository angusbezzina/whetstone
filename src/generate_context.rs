//! Generate agent context files from approved Whetstone rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml and renders Tera
//! templates for each requested format (CLAUDE.md, AGENTS.md, .cursorrules,
//! copilot-instructions.md, .windsurfrules, codex.md).

use anyhow::Result;
use serde_json::Value;
use std::path::Path;
use tera::Context;

use crate::config::WhetstoneConfig;
use crate::rules::{self, ApprovedRule};
use crate::templates::{build_tera, render};

/// Default formats if none specified in config.
const DEFAULT_FORMATS: &[&str] = &["agents.md"];

/// All supported format names and the template + filename they resolve to.
const FORMAT_SPECS: &[(&str, &str, &str)] = &[
    ("claude.md", "claude_md.tera", "CLAUDE.md"),
    ("agents.md", "agents_md.tera", "AGENTS.md"),
    (".cursorrules", "cursorrules.tera", ".cursorrules"),
    (
        "copilot-instructions.md",
        "copilot_md.tera",
        ".github/copilot-instructions.md",
    ),
    (".windsurfrules", "windsurfrules.tera", ".windsurfrules"),
    ("codex.md", "codex_md.tera", "codex.md"),
];

pub fn generate_context(
    project_dir: &Path,
    formats_filter: Option<&str>,
    lang_filter: Option<&str>,
    dry_run: bool,
    personal_output: bool,
) -> Result<Value> {
    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let config = WhetstoneConfig::load(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let (approved, output_dir, warnings): (Vec<ApprovedRule>, _, Vec<String>) = if personal_output {
        let (rules, warns) = crate::layers::load_personal_only(project_dir, lang_filter);
        (rules, paths.personal_context(), warns)
    } else if whetstone_config_exists {
        let merged = crate::layers::resolve_merged(project_dir, lang_filter, true, false, false);
        let approved = merged.merged.into_iter().map(|lr| lr.rule).collect();
        (
            approved,
            paths.whetstone_dir.join("context"),
            merged.warnings,
        )
    } else {
        let (approved, warns) = rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
        (approved, paths.whetstone_dir.join("context"), warns)
    };

    if approved.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": [],
            "warnings": ["No approved rules found. Run 'wh doctor' to extract and approve rules."],
            "next_command": "wh doctor",
        }));
    }

    let formats: Vec<String> = if let Some(filter) = formats_filter {
        filter.split(',').map(|s| s.trim().to_lowercase()).collect()
    } else if !config.generate.formats.is_empty() {
        config.generate.formats.clone()
    } else {
        DEFAULT_FORMATS.iter().map(|s| s.to_string()).collect()
    };

    for f in &formats {
        if FORMAT_SPECS.iter().all(|(name, _, _)| *name != f.as_str()) {
            let valid: Vec<&str> = FORMAT_SPECS.iter().map(|(n, _, _)| *n).collect();
            return Ok(serde_json::json!({
                "status": "error",
                "error": format!("Unknown format: '{f}'. Valid: {}", valid.join(", ")),
            }));
        }
    }

    let tera = build_tera();
    let (use_rules, avoid_rules) = split_rules(&approved);
    let deps = dedup_sorted_deps(&approved);
    let timestamp = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut ctx = Context::new();
    ctx.insert("use_rules", &use_rules);
    ctx.insert("avoid_rules", &avoid_rules);
    ctx.insert("deps", &deps);
    ctx.insert("timestamp", &timestamp);

    let mut generated = Vec::new();
    let mut skipped = Vec::new();

    for format in &formats {
        let (template, filename) = lookup_format(format).unwrap();
        let rendered = render(&tera, template, &ctx);
        let output_path = output_dir.join(filename);

        if dry_run {
            generated.push(serde_json::json!({
                "format": format,
                "path": output_path.display().to_string(),
                "lines": rendered.lines().count(),
                "dry_run": true,
            }));
            continue;
        }

        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&output_path, &rendered) {
            Ok(()) => {
                generated.push(serde_json::json!({
                    "format": format,
                    "path": output_path.display().to_string(),
                    "lines": rendered.lines().count(),
                }));
            }
            Err(e) => {
                skipped.push(format!("Failed to write {}: {e}", output_path.display()));
            }
        }
    }

    let next_command = if generated.is_empty() {
        "wh doctor"
    } else {
        "wh tests"
    };

    Ok(serde_json::json!({
        "status": "ok",
        "generated": generated,
        "skipped": skipped,
        "rules_count": approved.len(),
        "dependencies": deps,
        "formats": formats,
        "warnings": warnings,
        "next_command": next_command,
    }))
}

fn lookup_format(name: &str) -> Option<(&'static str, &'static str)> {
    FORMAT_SPECS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, tpl, file)| (*tpl, *file))
}

#[derive(serde::Serialize)]
struct ContentRule {
    id: String,
    description: String,
    source_url: String,
    severity: String,
    category: String,
    source_name: String,
    pass_examples: Vec<String>,
    fail_examples: Vec<String>,
}

fn split_rules(approved: &[ApprovedRule]) -> (Vec<ContentRule>, Vec<ContentRule>) {
    let mut use_rules = Vec::new();
    let mut avoid_rules = Vec::new();

    for rule in approved {
        let pass_examples: Vec<String> = rule
            .golden_examples
            .iter()
            .filter(|e| e.verdict == "pass")
            .map(|e| e.code.clone())
            .collect();
        let fail_examples: Vec<String> = rule
            .golden_examples
            .iter()
            .filter(|e| e.verdict == "fail")
            .map(|e| e.code.clone())
            .collect();

        let cr = ContentRule {
            id: rule.id.clone(),
            description: rule.description.clone(),
            source_url: rule.source_url.clone(),
            severity: rule.severity.clone(),
            category: rule.category.clone(),
            source_name: rule.source_name.clone(),
            pass_examples,
            fail_examples,
        };

        if matches!(rule.category.as_str(), "migration" | "breaking-change") {
            avoid_rules.push(cr);
        } else {
            use_rules.push(cr);
        }
    }

    (use_rules, avoid_rules)
}

fn dedup_sorted_deps(approved: &[ApprovedRule]) -> Vec<String> {
    let mut names: Vec<String> = approved.iter().map(|r| r.source_name.clone()).collect();
    names.sort();
    names.dedup();
    names
}
