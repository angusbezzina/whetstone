//! `wh extract` — walk the extraction worklist and submit candidate rules.
//!
//! Replaces the previous propose-bundle flow. Two modes:
//!
//! - `wh extract` (no subcommand): thin wrapper over `worklist::load` that
//!   prints the top dep with ranked sources, section summaries, quota, and
//!   a concrete next-step hint for the agent.
//! - `wh extract submit <bundle.yaml>`: read a bundle YAML, validate that
//!   every rule's id is unique against the existing ruleset, and write the
//!   rules out as `status: candidate` under
//!   `whetstone/rules/<lang>/<dep>.yaml`.
//!
//! A bundle looks like:
//! ```yaml
//! dependency: fastapi
//! language: python
//! source:
//!   name: fastapi
//!   docs_url: https://fastapi.tiangolo.com
//!   version: 0.115.0
//!   registry: pypi
//! rules:
//!   - id: fastapi.async-routes
//!     severity: must
//!     confidence: high
//!     category: convention
//!     description: "..."
//!     source_url: "..."
//!     signals: [...]
//!     golden_examples: [...]
//! ```
//!
//! On id collision with an existing rule (any status), the command errors
//! out naming the colliding id and stops — no partial writes.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::load_rule_files;
use crate::worklist;

// ── Bundle shape ──

#[derive(Debug, Deserialize)]
pub struct Bundle {
    pub dependency: String,
    pub language: String,
    #[serde(default)]
    pub source: Option<YamlValue>,
    #[serde(default)]
    pub rules: Vec<YamlValue>,
}

// ── Top-level entrypoints ──

/// Default mode: render the worklist for interactive extraction.
pub fn show_worklist(project_dir: &Path, dep: Option<&str>, lang: Option<&str>) -> Result<Value> {
    let handoff = worklist::load(project_dir)?;
    let wl = handoff
        .get("worklist")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let filtered = worklist::filter(&wl, dep, lang);
    Ok(json!({
        "status": "ok",
        "generated_at": handoff.get("generated_at"),
        "trigger": handoff.get("trigger"),
        "total": filtered.len(),
        "entries": filtered,
        "next_command": "Pick the first `ready_now` entry, extract rules, and `wh extract submit <bundle.yaml>`",
    }))
}

/// Submit a bundle of candidate rules to the project rules tree.
pub fn submit(project_dir: &Path, bundle_path: &Path) -> Result<Value> {
    let text = fs::read_to_string(bundle_path)
        .map_err(|e| anyhow!("cannot read bundle {}: {e}", bundle_path.display()))?;
    let bundle: Bundle = serde_yaml::from_str(&text)
        .map_err(|e| anyhow!("invalid bundle YAML {}: {e}", bundle_path.display()))?;

    if bundle.dependency.trim().is_empty() {
        return Err(anyhow!("bundle missing `dependency`"));
    }
    if bundle.language.trim().is_empty() {
        return Err(anyhow!("bundle missing `language`"));
    }
    if !matches!(bundle.language.as_str(), "python" | "typescript" | "rust") {
        return Err(anyhow!(
            "bundle language must be one of python|typescript|rust (got `{}`)",
            bundle.language
        ));
    }
    if bundle.rules.is_empty() {
        return Err(anyhow!("bundle carries zero rules"));
    }

    // Collect every existing rule id across writable layers.
    let existing_ids = collect_existing_rule_ids(project_dir);

    let mut candidate_ids: Vec<String> = Vec::new();
    for raw in &bundle.rules {
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("bundle contains a rule without `id`"))?;
        if existing_ids.contains(id) {
            return Err(anyhow!(
                "rule id `{id}` already exists in the project ruleset — rename the candidate or remove the existing rule"
            ));
        }
        candidate_ids.push(id.to_string());
    }

    // Assemble the destination file in YAML form.
    let dest = destination_path(project_dir, &bundle.language, &bundle.dependency);

    let mut rules_out: Vec<YamlValue> = Vec::with_capacity(bundle.rules.len());
    for raw in &bundle.rules {
        let mut rule = raw.clone();
        if let YamlValue::Mapping(ref mut map) = rule {
            map.insert(
                YamlValue::String("status".into()),
                YamlValue::String("candidate".into()),
            );
            map.insert(YamlValue::String("approved".into()), YamlValue::Bool(false));
        }
        rules_out.push(rule);
    }

    let mut top: serde_yaml::Mapping = serde_yaml::Mapping::new();
    if let Some(src) = bundle.source {
        top.insert(YamlValue::String("source".into()), src);
    } else {
        top.insert(
            YamlValue::String("source".into()),
            YamlValue::Mapping({
                let mut m = serde_yaml::Mapping::new();
                m.insert(
                    YamlValue::String("name".into()),
                    YamlValue::String(bundle.dependency.clone()),
                );
                m
            }),
        );
    }
    top.insert(
        YamlValue::String("rules".into()),
        YamlValue::Sequence(rules_out),
    );
    let body = serde_yaml::to_string(&YamlValue::Mapping(top))?;

    // If the destination already exists, refuse — submit should be additive only.
    if dest.exists() {
        return Err(anyhow!(
            "destination {} already exists; delete or rename before resubmitting",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest, body)?;

    Ok(json!({
        "status": "ok",
        "bundle": bundle_path.display().to_string(),
        "wrote": dest.display().to_string(),
        "candidate_ids": candidate_ids,
        "next_command": "wh approve --all --confidence high",
    }))
}

// ── Helpers ──

fn collect_existing_rule_ids(project_dir: &Path) -> HashSet<String> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut out = HashSet::new();
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        let (files, _) = load_rule_files(dir);
        for lrf in files {
            for rule in &lrf.rule_file.rules {
                if !rule.id.is_empty() {
                    out.insert(rule.id.clone());
                }
            }
        }
    }
    out
}

fn destination_path(project_dir: &Path, language: &str, dependency: &str) -> PathBuf {
    project_dir
        .join("whetstone")
        .join("rules")
        .join(language)
        .join(format!("{dependency}.yaml"))
}
