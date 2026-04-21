//! Generate linter configuration overlays from approved rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml, mines `lint_proxy`
//! signals, and writes one config per supported linter:
//! - Python: `whetstone/lint/ruff.whetstone.toml`
//! - TypeScript: `whetstone/lint/biome.whetstone.json`
//! - Rust: `whetstone/lint/clippy.whetstone.toml`
//!
//! Split out from `generate_tests` so `wh tests` stays focused on eval
//! harnesses and `wh lint` can evolve independently (bead whetstone-5f4).

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

use crate::rules::{self, ApprovedRule};
use crate::templates::{build_tera, render};

pub fn generate_lint(
    project_dir: &Path,
    lang_filter: Option<&str>,
    dry_run: bool,
    personal_output: bool,
) -> Result<Value> {
    let project_initialized = crate::layers::project_is_initialized(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let (approved, warnings, output_base): (Vec<ApprovedRule>, Vec<String>, PathBuf) =
        if personal_output {
            let (rules, warns) = crate::layers::load_personal_only(project_dir, lang_filter);
            (rules, warns, paths.personal_dir.clone())
        } else if project_initialized {
            let merged =
                crate::layers::resolve_merged(project_dir, lang_filter, true, false, false);
            let approved = merged.merged.into_iter().map(|lr| lr.rule).collect();
            (approved, merged.warnings, paths.whetstone_dir.clone())
        } else {
            let (approved, warns) =
                rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
            (approved, warns, paths.whetstone_dir.clone())
        };

    if approved.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": {"lint_configs": []},
            "warnings": ["No approved rules found. Run 'wh init' to extract and approve rules."],
            "next_command": "wh init",
        }));
    }

    let tera = build_tera();

    let mut by_language: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in &approved {
        by_language
            .entry(rule.language.clone())
            .or_default()
            .push(rule);
    }

    let mut lint_configs: Vec<Value> = Vec::new();
    let mut all_warnings: Vec<String> = warnings;

    for (language, rules) in &by_language {
        match language.as_str() {
            "python" => {
                lint_configs.extend(generate_python_lint(&tera, rules, &output_base, dry_run));
            }
            "typescript" => {
                lint_configs.extend(generate_typescript_lint(&tera, rules, &output_base, dry_run));
            }
            "rust" => {
                lint_configs.extend(generate_rust_lint(&tera, rules, &output_base, dry_run));
            }
            _ => {
                all_warnings.push(format!("Skipping unsupported language: {language}"));
            }
        }
    }

    Ok(serde_json::json!({
        "status": "ok",
        "generated": {
            "lint_configs": lint_configs,
        },
        "rules_count": approved.len(),
        "languages": by_language.keys().collect::<Vec<_>>(),
        "warnings": all_warnings,
    }))
}

// ── Per-language emitters ──

fn generate_python_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut out = Vec::new();
    let ruff_rules = extract_lint_proxy_codes(rules, "ruff");
    if ruff_rules.is_empty() {
        return out;
    }
    let mut ctx = Context::new();
    ctx.insert("codes", &ruff_rules);
    let content = render(tera, "ruff_config.tera", &ctx);
    let path = output_base.join("lint").join("ruff.whetstone.toml");
    if write_generated(&path, &content, dry_run) {
        out.push(serde_json::json!({
            "path": path.display().to_string(),
            "type": "ruff",
            "rules": ruff_rules,
        }));
    }
    out
}

fn generate_typescript_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut out = Vec::new();
    let biome_rules = extract_lint_proxy_codes(rules, "biome");
    if biome_rules.is_empty() {
        return out;
    }
    let grouped = group_biome_rules(&biome_rules);
    let mut ctx = Context::new();
    ctx.insert("groups", &grouped);
    let content = render(tera, "biome_config.tera", &ctx);
    let path = output_base.join("lint").join("biome.whetstone.json");
    if write_generated(&path, &content, dry_run) {
        out.push(serde_json::json!({
            "path": path.display().to_string(),
            "type": "biome",
            "rules": biome_rules,
        }));
    }
    out
}

fn generate_rust_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut out = Vec::new();
    let clippy_rules = extract_lint_proxy_codes(rules, "clippy");
    if clippy_rules.is_empty() {
        return out;
    }
    let mut ctx = Context::new();
    ctx.insert("lints", &clippy_rules);
    let content = render(tera, "clippy_config.tera", &ctx);
    let path = output_base.join("lint").join("clippy.whetstone.toml");
    if write_generated(&path, &content, dry_run) {
        out.push(serde_json::json!({
            "path": path.display().to_string(),
            "type": "clippy",
            "rules": clippy_rules,
        }));
    }
    out
}

// ── Helpers (kept local — duplicated from generate_tests for isolation) ──

fn extract_lint_proxy_codes(rules: &[&ApprovedRule], linter: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for rule in rules {
        for signal in &rule.signals {
            if signal.strategy != "lint_proxy" {
                continue;
            }
            let desc = signal.description.to_lowercase();
            if !desc.contains(linter) {
                continue;
            }
            let parts: Vec<&str> = signal.description.split_whitespace().collect();
            for (i, part) in parts.iter().enumerate() {
                if part.to_lowercase() == linter && i + 1 < parts.len() {
                    codes.push(parts[i + 1].to_string());
                }
            }
        }
    }
    codes.sort();
    codes.dedup();
    codes
}

fn group_biome_rules(rules: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rule in rules {
        if let Some((category, name)) = rule.split_once('/') {
            out.entry(category.to_string())
                .or_default()
                .push(name.to_string());
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

fn write_generated(path: &Path, content: &str, dry_run: bool) -> bool {
    if dry_run {
        return true;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, content).is_ok()
}
