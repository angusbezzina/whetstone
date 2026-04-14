//! Review and apply workflow for rule lifecycle transitions.
//!
//! Commands exposed via the CLI:
//! - `wh review` — list rules grouped by lifecycle status
//! - `wh review show` — show full context for a single rule
//! - `wh review queue` — build a review queue from extraction-handoff +
//!   refresh-diff artifacts (drives 3C.2)
//! - `wh apply` — transition a rule: approve, deny, deprecate, or supersede
//!   without hand-editing YAML
//!
//! Every transition is appended to `whetstone/.state/review-log.jsonl` so
//! there is a durable audit trail for "who approved rule X, when, and why".

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};

use crate::layers::LayerPaths;
use crate::rules::{load_rule_files, RuleFile};

const REVIEW_LOG: &str = "review-log.jsonl";

// ── Types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Approve,
    Deny,
    Deprecate,
    Supersede, // deprecate + set superseded_by
}

impl Transition {
    fn target_status(&self) -> &'static str {
        match self {
            Transition::Approve => "approved",
            Transition::Deny => "denied",
            Transition::Deprecate | Transition::Supersede => "deprecated",
        }
    }
}

pub struct ApplyOptions<'a> {
    pub project_dir: &'a Path,
    pub rule_id: &'a str,
    pub transition: Transition,
    pub reason: Option<&'a str>,
    pub superseded_by: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub dry_run: bool,
}

pub struct ReviewListOptions<'a> {
    pub project_dir: &'a Path,
    pub status_filter: Option<&'a str>,
    pub lang_filter: Option<&'a str>,
}

// ── List ──

/// Enumerate rules across all writable layers (personal + project), grouped
/// by lifecycle status so reviewers can see candidates / approved / denied
/// / deprecated at a glance.
pub fn list(opts: ReviewListOptions<'_>) -> Result<Value> {
    let paths = LayerPaths::for_project(opts.project_dir);
    let (project_files, mut warnings) = load_rule_files(&paths.project_rules_dir);
    let (personal_files, mut personal_warnings) = load_rule_files(&paths.personal_rules_dir);
    warnings.append(&mut personal_warnings);

    let mut by_status: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    collect(&mut by_status, &project_files, "project", opts.lang_filter);
    collect(&mut by_status, &personal_files, "personal", opts.lang_filter);

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
        "next_command": "wh review show <rule-id> | wh apply <rule-id> --approve",
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
            let status = rule
                .status
                .clone()
                .unwrap_or_else(|| if rule.approved { "approved".into() } else { "candidate".into() });
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
                "approved_at": rule.approved_at,
                "proposed_at": rule.proposed_at,
                "proposed_by": rule.proposed_by,
                "denied_reason": rule.denied_reason,
                "deprecated_reason": rule.deprecated_reason,
                "superseded_by": rule.superseded_by,
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
            "risk": rule.risk,
            "linter_gap": rule.linter_gap,
            "status": rule.status.as_deref().unwrap_or(if rule.approved { "approved" } else { "candidate" }),
            "approved": rule.approved,
            "approved_at": rule.approved_at,
            "proposed_at": rule.proposed_at,
            "proposed_by": rule.proposed_by,
            "denied_reason": rule.denied_reason,
            "deprecated_reason": rule.deprecated_reason,
            "superseded_by": rule.superseded_by,
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

// ── Apply ──

pub fn apply(opts: ApplyOptions<'_>) -> Result<Value> {
    let found = find_rule(opts.project_dir, opts.rule_id)?;
    let rule = found
        .file
        .rules
        .iter()
        .find(|r| r.id == opts.rule_id)
        .ok_or_else(|| anyhow!("rule {} not found after load", opts.rule_id))?;

    let current_status = rule
        .status
        .clone()
        .unwrap_or_else(|| if rule.approved { "approved".into() } else { "candidate".into() });
    let target_status = opts.transition.target_status();

    validate_transition(&current_status, target_status)?;
    validate_reasons(opts.transition, opts.reason, opts.superseded_by)?;

    if opts.dry_run {
        return Ok(json!({
            "status": "ok",
            "action": "dry_run",
            "rule_id": opts.rule_id,
            "from": current_status,
            "to": target_status,
            "file": found.path.display().to_string(),
            "reason": opts.reason,
            "superseded_by": opts.superseded_by,
            "note": "No files were modified. Re-run without --dry-run to commit.",
        }));
    }

    let now = chrono::Utc::now().to_rfc3339();
    update_yaml_file(
        &found.path,
        opts.rule_id,
        opts.transition,
        &now,
        opts.reason,
        opts.superseded_by,
    )?;

    append_audit_entry(
        opts.project_dir,
        AuditEntry {
            timestamp: now.clone(),
            rule_id: opts.rule_id.to_string(),
            from_status: current_status.clone(),
            to_status: target_status.to_string(),
            reason: opts.reason.map(String::from),
            superseded_by: opts.superseded_by.map(String::from),
            actor: opts.actor.map(String::from),
            file: found.path.display().to_string(),
        },
    )?;

    Ok(json!({
        "status": "ok",
        "action": "applied",
        "rule_id": opts.rule_id,
        "from": current_status,
        "to": target_status,
        "file": found.path.display().to_string(),
        "applied_at": now,
        "reason": opts.reason,
        "superseded_by": opts.superseded_by,
        "audit_log": audit_log_path(opts.project_dir).display().to_string(),
    }))
}

// ── Batch apply ──

#[derive(Debug, Deserialize)]
pub struct BatchEntry {
    pub rule_id: String,
    pub action: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub superseded_by: Option<String>,
}

pub fn apply_batch(project_dir: &Path, batch_file: &Path, dry_run: bool) -> Result<Value> {
    let text = fs::read_to_string(batch_file)?;
    let entries: Vec<BatchEntry> = serde_json::from_str(&text)
        .map_err(|e| anyhow!("invalid batch file: {e}"))?;

    let mut applied = Vec::new();
    let mut failed = Vec::new();
    for entry in &entries {
        let transition = match entry.action.as_str() {
            "approve" => Transition::Approve,
            "deny" => Transition::Deny,
            "deprecate" => Transition::Deprecate,
            "supersede" => Transition::Supersede,
            other => {
                failed.push(json!({
                    "rule_id": entry.rule_id,
                    "error": format!("unknown action: {other}"),
                }));
                continue;
            }
        };
        let result = apply(ApplyOptions {
            project_dir,
            rule_id: &entry.rule_id,
            transition,
            reason: entry.reason.as_deref(),
            superseded_by: entry.superseded_by.as_deref(),
            actor: Some("batch"),
            dry_run,
        });
        match result {
            Ok(v) => applied.push(v),
            Err(e) => failed.push(json!({
                "rule_id": entry.rule_id,
                "error": e.to_string(),
            })),
        }
    }

    Ok(json!({
        "status": if failed.is_empty() { "ok" } else { "partial" },
        "applied": applied,
        "failed": failed,
        "total": entries.len(),
        "dry_run": dry_run,
    }))
}

// ── Queue from handoff + refresh artifacts (3C.2) ──

pub fn queue(project_dir: &Path) -> Result<Value> {
    let state_dir = project_dir.join("whetstone").join(".state");
    let handoff_path = state_dir.join("extraction-handoff.json");
    let diff_path = state_dir.join("refresh-diff.json");

    let handoff = load_json_or_null(&handoff_path);
    let diff = load_json_or_null(&diff_path);

    let candidate_deps = handoff
        .get("candidates")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let changed = diff
        .get("changed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let removed = diff
        .get("removed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let list_result = list(ReviewListOptions {
        project_dir,
        status_filter: Some("candidate"),
        lang_filter: None,
    })?;
    let pending_candidates = list_result
        .get("rules")
        .and_then(|r| r.get("candidate"))
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));

    let stale_rules = collect_stale_rules(&changed);

    let total = pending_candidates
        .as_array()
        .map(|v| v.len())
        .unwrap_or(0)
        + stale_rules.len()
        + removed.len();

    Ok(json!({
        "status": "ok",
        "summary": {
            "candidates_pending_review": pending_candidates.as_array().map(|v| v.len()).unwrap_or(0),
            "candidate_deps_awaiting_extraction": candidate_deps.len(),
            "rules_affected_by_source_change": stale_rules.len(),
            "deps_removed_from_manifest": removed.len(),
            "total_actions": total,
        },
        "pending_candidates": pending_candidates,
        "awaiting_extraction": candidate_deps,
        "stale_rules": stale_rules,
        "removed_deps": removed,
        "next_command": "wh review show <rule-id> | wh apply <rule-id> --approve|--deny|--deprecate",
    }))
}

fn collect_stale_rules(changed: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for entry in changed {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let lang = entry.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let affected: Vec<String> = entry
            .get("affected_rule_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for rid in affected {
            out.push(json!({
                "rule_id": rid,
                "dependency": name,
                "language": lang,
                "reason": "source content changed; re-validate or deprecate",
            }));
        }
    }
    out
}

// ── Internals ──

struct FoundRule {
    path: PathBuf,
    file: RuleFile,
}

fn find_rule(project_dir: &Path, rule_id: &str) -> Result<FoundRule> {
    let paths = LayerPaths::for_project(project_dir);
    for dir in [&paths.personal_rules_dir, &paths.project_rules_dir] {
        let (files, _) = load_rule_files(dir);
        for lrf in files {
            if lrf.rule_file.rules.iter().any(|r| r.id == rule_id) {
                return Ok(FoundRule {
                    path: PathBuf::from(&lrf.file_path),
                    file: lrf.rule_file,
                });
            }
        }
    }
    Err(anyhow!(
        "rule '{rule_id}' not found under whetstone/rules/ or whetstone/.personal/rules/"
    ))
}

fn validate_transition(from: &str, to: &str) -> Result<()> {
    use std::collections::HashSet;
    let allowed: HashSet<(&str, &str)> = [
        ("candidate", "approved"),
        ("candidate", "denied"),
        ("candidate", "deprecated"),
        ("approved", "deprecated"),
    ]
    .into_iter()
    .collect();
    if !allowed.contains(&(from, to)) {
        return Err(anyhow!(
            "illegal transition {from} → {to}. Denied and deprecated are terminal states."
        ));
    }
    Ok(())
}

fn validate_reasons(
    transition: Transition,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> Result<()> {
    match transition {
        Transition::Deny if reason.is_none() => {
            Err(anyhow!("--deny requires --reason to persist audit context"))
        }
        Transition::Deprecate if reason.is_none() => Err(anyhow!(
            "--deprecate requires --reason (explain why the rule is retired)"
        )),
        Transition::Supersede if superseded_by.is_none() => Err(anyhow!(
            "--supersede requires --superseded-by <rule-id>"
        )),
        _ => Ok(()),
    }
}

/// Rewrite the rule file in place, updating only the fields that belong to
/// the named rule. Unrelated rules and source metadata pass through untouched.
fn update_yaml_file(
    path: &Path,
    rule_id: &str,
    transition: Transition,
    now_iso: &str,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let mut root: YamlValue = serde_yaml::from_str(&text)
        .map_err(|e| anyhow!("failed to parse {}: {e}", path.display()))?;

    let rules = root
        .get_mut("rules")
        .and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| anyhow!("{} has no `rules:` sequence", path.display()))?;

    let mut found = false;
    for rule in rules.iter_mut() {
        let id_matches = rule
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == rule_id)
            .unwrap_or(false);
        if !id_matches {
            continue;
        }
        found = true;

        let map = rule
            .as_mapping_mut()
            .ok_or_else(|| anyhow!("rule {rule_id} is not a mapping"))?;

        let target = transition.target_status();
        map.insert(YamlValue::from("status"), YamlValue::from(target));

        match transition {
            Transition::Approve => {
                map.insert(YamlValue::from("approved"), YamlValue::from(true));
                map.insert(
                    YamlValue::from("approved_at"),
                    YamlValue::from(now_iso),
                );
                // Approving clears any prior soft-denial fields.
                map.remove(YamlValue::from("denied_reason"));
            }
            Transition::Deny => {
                map.insert(YamlValue::from("approved"), YamlValue::from(false));
                if let Some(r) = reason {
                    map.insert(
                        YamlValue::from("denied_reason"),
                        YamlValue::from(r),
                    );
                }
            }
            Transition::Deprecate | Transition::Supersede => {
                map.insert(YamlValue::from("approved"), YamlValue::from(false));
                if let Some(r) = reason {
                    map.insert(
                        YamlValue::from("deprecated_reason"),
                        YamlValue::from(r),
                    );
                }
                if let Some(s) = superseded_by {
                    map.insert(
                        YamlValue::from("superseded_by"),
                        YamlValue::from(s),
                    );
                }
            }
        }
        break;
    }

    if !found {
        return Err(anyhow!(
            "rule {rule_id} not found in {}",
            path.display()
        ));
    }

    let serialized = serde_yaml::to_string(&root)
        .map_err(|e| anyhow!("failed to serialize {}: {e}", path.display()))?;
    fs::write(path, serialized)?;
    Ok(())
}

// ── Audit log ──

#[derive(Serialize, Deserialize)]
struct AuditEntry {
    timestamp: String,
    rule_id: String,
    from_status: String,
    to_status: String,
    reason: Option<String>,
    superseded_by: Option<String>,
    actor: Option<String>,
    file: String,
}

fn audit_log_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("whetstone")
        .join(".state")
        .join(REVIEW_LOG)
}

fn append_audit_entry(project_dir: &Path, entry: AuditEntry) -> Result<()> {
    let path = audit_log_path(project_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    use std::io::Write as _;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let line = serde_json::to_string(&entry)?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn load_json_or_null(path: &Path) -> Value {
    if !path.exists() {
        return Value::Null;
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

// ── Human output ──

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
    fn validate_transition_forbids_going_backwards() {
        assert!(validate_transition("approved", "candidate").is_err());
        assert!(validate_transition("denied", "approved").is_err());
        assert!(validate_transition("deprecated", "approved").is_err());
    }

    #[test]
    fn validate_transition_allows_happy_path() {
        assert!(validate_transition("candidate", "approved").is_ok());
        assert!(validate_transition("candidate", "denied").is_ok());
        assert!(validate_transition("approved", "deprecated").is_ok());
    }

    #[test]
    fn validate_reasons_requires_deny_reason() {
        assert!(validate_reasons(Transition::Deny, None, None).is_err());
        assert!(validate_reasons(Transition::Deny, Some("bad"), None).is_ok());
    }

    #[test]
    fn validate_reasons_requires_supersede_target() {
        assert!(validate_reasons(Transition::Supersede, Some("r"), None).is_err());
        assert!(validate_reasons(Transition::Supersede, Some("r"), Some("other.id")).is_ok());
    }
}
