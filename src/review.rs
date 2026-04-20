//! Rule review helpers used by `wh review`.
//!
//! The lean refactor (bead whetstone-aww) reduced the command surface to:
//! - `wh review` — list rules grouped by lifecycle status
//! - `wh review show <id>` — full context for a single rule
//! - `wh review worklist` — render the dependency-scoped extraction worklist
//!
//! Apply/queue/diff flows were removed; the later beads introduce
//! `wh extract` (submit candidates) and `wh approve` (candidate → approved)
//! which own the lifecycle transitions instead.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::Path;

use crate::layers::LayerPaths;
use crate::rules::{load_rule_files, RuleFile};

pub struct ReviewListOptions<'a> {
    pub project_dir: &'a Path,
    pub status_filter: Option<&'a str>,
    pub lang_filter: Option<&'a str>,
}

// ── List ──

/// Enumerate rules across all writable layers (personal + project), grouped
/// by lifecycle status so reviewers can see candidates / approved at a glance.
pub fn list(opts: ReviewListOptions<'_>) -> Result<Value> {
    let paths = LayerPaths::for_project(opts.project_dir);
    let (project_files, mut warnings) = load_rule_files(&paths.project_rules_dir);
    let (personal_files, mut personal_warnings) = load_rule_files(&paths.personal_rules_dir);
    warnings.append(&mut personal_warnings);

    let mut by_status: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    collect(&mut by_status, &project_files, "project", opts.lang_filter);
    collect(
        &mut by_status,
        &personal_files,
        "personal",
        opts.lang_filter,
    );

    if let Some(filter) = opts.status_filter {
        by_status.retain(|k, _| k == filter);
    }

    let total: usize = by_status.values().map(|v| v.len()).sum();

    Ok(json!({
        "status": "ok",
        "summary": {
            "total": total,
            "by_status": by_status.iter().map(|(k, v)| (k.clone(), Value::from(v.len()))).collect::<serde_json::Map<_, _>>(),
        },
        "rules": by_status,
        "warnings": warnings,
        "next_command": "wh review show <rule-id> | wh approve <rule-id>",
    }))
}

fn collect(
    out: &mut std::collections::BTreeMap<String, Vec<Value>>,
    files: &[crate::rules::LoadedRuleFile],
    layer: &str,
    lang_filter: Option<&str>,
) {
    for lrf in files {
        if let Some(filter) = lang_filter {
            if lrf.language.as_deref() != Some(filter) {
                continue;
            }
        }
        for rule in &lrf.rule_file.rules {
            let status = rule.status.clone().unwrap_or_else(|| {
                if rule.approved {
                    "approved".into()
                } else {
                    "candidate".into()
                }
            });
            out.entry(status).or_default().push(json!({
                "id": rule.id,
                "severity": rule.severity,
                "category": rule.category,
                "description": first_line(rule.description.as_deref().unwrap_or("")),
                "file": lrf.file_path,
                "language": lrf.language,
                "layer": layer,
                "source_name": lrf.rule_file.source.name,
                "source_url": rule.source_url,
                "approved": rule.approved,
            }));
        }
    }
}

// ── Show ──

pub fn show(project_dir: &Path, rule_id: &str) -> Result<Value> {
    let found = find_rule(project_dir, rule_id)?;
    let rule = found
        .file
        .rules
        .iter()
        .find(|r| r.id == rule_id)
        .ok_or_else(|| anyhow!("rule {rule_id} vanished between load and lookup"))?;

    Ok(json!({
        "status": "ok",
        "rule": {
            "id": rule.id,
            "severity": rule.severity,
            "confidence": rule.confidence,
            "category": rule.category,
            "description": rule.description,
            "source_url": rule.source_url,
            "source_quote": rule.source_quote,
            "status": rule.status.as_deref().unwrap_or(if rule.approved { "approved" } else { "candidate" }),
            "approved": rule.approved,
            "signals": rule.signals.iter().map(|s| json!({
                "id": s.id,
                "strategy": s.strategy,
                "description": s.description,
                "weight": s.weight,
                "match": s.match_pattern,
            })).collect::<Vec<_>>(),
            "golden_examples": rule.golden_examples.iter().map(|e| json!({
                "verdict": e.verdict,
                "reason": e.reason,
                "code": e.code,
            })).collect::<Vec<_>>(),
        },
        "file": found.path.display().to_string(),
        "source_name": found.file.source.name,
    }))
}

// ── Internals ──

struct FoundRule {
    path: std::path::PathBuf,
    file: RuleFile,
}

fn find_rule(project_dir: &Path, rule_id: &str) -> Result<FoundRule> {
    let paths = LayerPaths::for_project(project_dir);
    for dir in [&paths.personal_rules_dir, &paths.project_rules_dir] {
        let (files, _) = load_rule_files(dir);
        for lrf in files {
            if lrf.rule_file.rules.iter().any(|r| r.id == rule_id) {
                return Ok(FoundRule {
                    path: std::path::PathBuf::from(&lrf.file_path),
                    file: lrf.rule_file,
                });
            }
        }
    }
    Err(anyhow!(
        "rule '{rule_id}' not found under whetstone/rules/ or whetstone/.personal/rules/"
    ))
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

// ── Human output ──

pub fn format_worklist(result: &Value) -> String {
    let mut out = String::new();
    let total = result.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
    out.push_str(&format!("wh review worklist: {total} entry/entries\n"));
    if let Some(entries) = result.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let lang = entry
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let priority = entry
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let remaining = entry
                .get("quota")
                .and_then(|q| q.get("remaining"))
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            let next = entry
                .get("next_step")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str(&format!(
                "  [{priority}] {name} ({lang}) — quota remaining: {remaining}\n    → {next}\n"
            ));
        }
    }
    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n{next}\n"));
    }
    out
}

pub fn format_list(result: &Value) -> String {
    let mut out = String::new();
    let total = result
        .get("summary")
        .and_then(|s| s.get("total"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    out.push_str(&format!("wh review: {total} rule(s)\n"));
    if let Some(rules) = result.get("rules").and_then(|v| v.as_object()) {
        for (status, arr) in rules {
            let list = arr.as_array().cloned().unwrap_or_default();
            out.push_str(&format!("  [{status}] {}\n", list.len()));
            for r in list {
                let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let desc = r.get("description").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!("    {id} — {desc}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_worklist_lists_entries_with_next_step() {
        let v = json!({
            "total": 1,
            "entries": [{
                "name": "fastapi",
                "language": "python",
                "priority": "ready_now",
                "quota": {"remaining": 5},
                "next_step": "Read the linked source",
            }],
            "next_command": "wh extract submit <bundle>",
        });
        let s = format_worklist(&v);
        assert!(s.contains("ready_now"), "{s}");
        assert!(s.contains("fastapi"), "{s}");
        assert!(s.contains("Read the linked source"), "{s}");
        assert!(s.contains("wh extract submit"), "{s}");
    }
}
