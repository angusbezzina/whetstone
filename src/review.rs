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

    let current_status = rule.status.clone().unwrap_or_else(|| {
        if rule.approved {
            "approved".into()
        } else {
            "candidate".into()
        }
    });
    let target_status = opts.transition.target_status();

    validate_transition(&current_status, target_status)?;
    validate_reasons(opts.transition, opts.reason, opts.superseded_by)?;
    if let Some(target_id) = opts.superseded_by {
        validate_supersedes_target(opts.project_dir, target_id)?;
    }

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
    let entries: Vec<BatchEntry> =
        serde_json::from_str(&text).map_err(|e| anyhow!("invalid batch file: {e}"))?;

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

// ── Candidate diff (3D.1.3) ──

/// Summarize what approving every currently-pending candidate would do:
/// how many would be added, how many existing approved rules in the same
/// dependency would be shadowed, and which existing approved rules are
/// not represented in the candidate set (advisory deprecations).
pub fn diff_candidates(
    project_dir: &Path,
    lang_filter: Option<&str>,
) -> Result<Value> {
    let paths = LayerPaths::for_project(project_dir);
    let (project_files, warnings) = load_rule_files(&paths.project_rules_dir);

    // Group rules by (language, dep_name).
    let mut approved_by_dep: std::collections::BTreeMap<(String, String), Vec<String>> =
        std::collections::BTreeMap::new();
    let mut candidates_by_dep: std::collections::BTreeMap<
        (String, String),
        Vec<Value>,
    > = std::collections::BTreeMap::new();
    let mut candidate_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for lrf in &project_files {
        if let Some(filter) = lang_filter {
            if lrf.language.as_deref() != Some(filter) {
                continue;
            }
        }
        let key = (
            lrf.language.clone().unwrap_or_default(),
            lrf.rule_file.source.name.clone(),
        );
        for r in &lrf.rule_file.rules {
            let st = r
                .status
                .clone()
                .unwrap_or_else(|| if r.approved { "approved".into() } else { "candidate".into() });
            match st.as_str() {
                "approved" => {
                    approved_by_dep
                        .entry(key.clone())
                        .or_default()
                        .push(r.id.clone());
                }
                "candidate" => {
                    candidate_ids.insert(r.id.clone());
                    candidates_by_dep
                        .entry(key.clone())
                        .or_default()
                        .push(json!({
                            "id": r.id,
                            "severity": r.severity,
                            "confidence": r.confidence,
                            "category": r.category,
                            "description": first_line(r.description.as_deref().unwrap_or("")),
                            "source_url": r.source_url,
                            "source_kind": r.source_kind,
                            "proposed_by": r.proposed_by,
                            "proposed_at": r.proposed_at,
                        }));
                }
                _ => {}
            }
        }
    }

    // Collect per-dep diff summaries.
    let mut per_dep: Vec<Value> = Vec::new();
    let all_keys: std::collections::BTreeSet<(String, String)> = approved_by_dep
        .keys()
        .chain(candidates_by_dep.keys())
        .cloned()
        .collect();
    let mut totals = DiffTotals::default();

    for key in all_keys {
        let candidates = candidates_by_dep.remove(&key).unwrap_or_default();
        let approved = approved_by_dep.remove(&key).unwrap_or_default();
        if candidates.is_empty() && approved.is_empty() {
            continue;
        }

        let would_add = candidates.len();
        let would_shadow: Vec<String> = candidates
            .iter()
            .filter_map(|c| c.get("id").and_then(|v| v.as_str()))
            .filter(|cid| approved.iter().any(|a| a == cid))
            .map(String::from)
            .collect();

        let advisory_deprecations: Vec<String> = approved
            .iter()
            .filter(|aid| !candidates.iter().any(|c| c["id"].as_str() == Some(aid)))
            .cloned()
            .collect();

        totals.added += would_add;
        totals.shadowed += would_shadow.len();
        totals.advisory_deprecations += advisory_deprecations.len();

        per_dep.push(json!({
            "language": key.0,
            "dependency": key.1,
            "candidates": candidates,
            "approved_existing": approved.len(),
            "would_shadow_existing_ids": would_shadow,
            "advisory_deprecations": advisory_deprecations,
        }));
    }

    Ok(json!({
        "status": "ok",
        "summary": {
            "total_candidates": totals.added,
            "candidate_deps": per_dep.len(),
            "would_shadow": totals.shadowed,
            "advisory_deprecations": totals.advisory_deprecations,
        },
        "per_dep": per_dep,
        "warnings": warnings,
        "next_command": "wh apply <rule-id> --approve | wh apply --batch <file.json>",
    }))
}

#[derive(Default)]
struct DiffTotals {
    added: usize,
    shadowed: usize,
    advisory_deprecations: usize,
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

    let total = pending_candidates.as_array().map(|v| v.len()).unwrap_or(0)
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
    // `candidate → deprecated` is intentionally disallowed: deprecation is the
    // retirement of something that was once adopted, so the rule must pass
    // through `approved` first. Denied and deprecated are terminal.
    let allowed: HashSet<(&str, &str)> = [
        ("candidate", "approved"),
        ("candidate", "denied"),
        ("approved", "deprecated"),
    ]
    .into_iter()
    .collect();
    if !allowed.contains(&(from, to)) {
        return Err(anyhow!(
            "illegal transition {from} → {to}. Denied and deprecated are terminal; deprecate only approved rules."
        ));
    }
    Ok(())
}

/// Ensure the rule id passed via `--superseded-by` actually exists in the
/// project's approved ruleset, so typos cannot silently write a dangling
/// pointer into the YAML + audit log.
fn validate_supersedes_target(project_dir: &Path, target_id: &str) -> Result<()> {
    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();

    if whetstone_config_exists {
        let merged = crate::layers::resolve_merged(project_dir, None, true, true, false);
        if merged.merged.iter().any(|lr| lr.rule.id == target_id) {
            return Ok(());
        }
    } else {
        let paths = LayerPaths::for_project(project_dir);
        for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
            let (files, _) = load_rule_files(dir);
            for lrf in &files {
                if lrf
                    .rule_file
                    .rules
                    .iter()
                    .any(|r| r.id == target_id && r.approved)
                {
                    return Ok(());
                }
            }
        }
    }
    Err(anyhow!(
        "--superseded-by target '{target_id}' not found in the current approved layered ruleset"
    ))
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
        Transition::Supersede if reason.is_none() => Err(anyhow!(
            "--supersede requires --reason (explain why the rule is being replaced)"
        )),
        Transition::Supersede if superseded_by.is_none() => {
            Err(anyhow!("--supersede requires --superseded-by <rule-id>"))
        }
        _ => Ok(()),
    }
}

/// Rewrite the rule file in place using line-based surgery. Only the fields
/// we actually need to mutate are touched; every comment, blank line,
/// indentation style, and quoting choice in the rest of the file survives.
///
/// The mutator expects the standard schema layout:
///   rules:
///     - id: <name>
///       status: ...
///       approved: ...
///       ...
/// i.e. each rule is a sequence entry under top-level `rules:`, with field
/// keys indented two spaces beyond the `- ` marker. The per-rule field
/// indent is detected from the first field after the `- id:` line so
/// non-standard indents still round-trip.
fn update_yaml_file(
    path: &Path,
    rule_id: &str,
    transition: Transition,
    now_iso: &str,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> Result<()> {
    // Parse once just to validate the rule exists and the file is legal YAML;
    // we throw this away and mutate the raw text to preserve formatting.
    let text = fs::read_to_string(path)?;
    let _parse_check: YamlValue = serde_yaml::from_str(&text)
        .map_err(|e| anyhow!("failed to parse {}: {e}", path.display()))?;

    let updates = build_updates(transition, now_iso, reason, superseded_by);
    let removes = build_removes(transition);

    let new_text = apply_rule_updates(&text, rule_id, &updates, &removes)
        .ok_or_else(|| anyhow!("rule {rule_id} not found in {}", path.display()))?;
    fs::write(path, new_text)?;
    Ok(())
}

type FieldUpdate = (&'static str, String);

fn build_updates(
    transition: Transition,
    now_iso: &str,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> Vec<FieldUpdate> {
    let mut updates: Vec<FieldUpdate> = Vec::new();
    updates.push(("status", transition.target_status().to_string()));
    match transition {
        Transition::Approve => {
            updates.push(("approved", "true".to_string()));
            updates.push(("approved_at", quote_yaml_string(now_iso)));
        }
        Transition::Deny => {
            updates.push(("approved", "false".to_string()));
            if let Some(r) = reason {
                updates.push(("denied_reason", quote_yaml_string(r)));
            }
        }
        Transition::Deprecate | Transition::Supersede => {
            updates.push(("approved", "false".to_string()));
            if let Some(r) = reason {
                updates.push(("deprecated_reason", quote_yaml_string(r)));
            }
            if let Some(s) = superseded_by {
                updates.push(("superseded_by", quote_yaml_string(s)));
            }
        }
    }
    updates
}

fn build_removes(transition: Transition) -> Vec<&'static str> {
    match transition {
        // Approving from candidate clears any prior denied_reason that a
        // reviewer might have queued before changing their mind.
        Transition::Approve => vec!["denied_reason"],
        _ => Vec::new(),
    }
}

/// Emit a YAML-safe scalar for a string value. Plain strings with no special
/// characters go through bare; everything else is double-quoted with the
/// minimum necessary escaping.
fn quote_yaml_string(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.starts_with([
            ' ', '\t', '-', '?', ':', '#', '&', '*', '!', '|', '>', '\'', '"',
        ])
        || value.chars().any(|c| {
            matches!(
                c,
                ':' | '#' | '\n' | '\t' | '{' | '}' | '[' | ']' | ',' | '`'
            )
        })
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        );
    if !needs_quotes {
        return value.to_string();
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Locate the target rule block and apply the field updates/removes in place,
/// returning the mutated text. Returns `None` if the rule id is not present.
fn apply_rule_updates(
    text: &str,
    rule_id: &str,
    updates: &[FieldUpdate],
    removes: &[&str],
) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();

    let (block_start, block_end, field_indent) = locate_rule_block(&lines, rule_id)?;
    let mut mutated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();

    // Remove requested fields first so an insert for the same name afterwards
    // lands in a predictable position.
    for key in removes {
        let mut current_end = mut_block_end(&mutated, block_start);
        remove_field_in_block(
            &mut mutated,
            block_start,
            &mut current_end,
            field_indent,
            key,
        );
    }

    for (key, value) in updates {
        let current_end = mut_block_end(&mutated, block_start);
        if !replace_field_in_block(
            &mut mutated,
            block_start,
            current_end,
            field_indent,
            key,
            value,
        ) {
            insert_field_at_end(&mut mutated, current_end, field_indent, key, value);
        }
    }

    let _ = block_end;
    Some(mutated.concat())
}

/// Find the slice of lines covering the rule whose `id:` equals `rule_id`.
/// Returns `(start_index, end_index_exclusive, field_indent)`, where
/// `field_indent` is the number of leading spaces on the rule's field lines.
fn locate_rule_block(lines: &[&str], rule_id: &str) -> Option<(usize, usize, usize)> {
    let start_re = Regex::id_line();
    let mut rule_start: Option<(usize, usize)> = None; // (index, id_line_indent)
    for (i, line) in lines.iter().enumerate() {
        if let Some(caps) = start_re.captures(line) {
            let leading = caps.get(1).map(|m| m.as_str().len()).unwrap_or(0);
            let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if id == rule_id {
                rule_start = Some((i, leading));
                break;
            }
        }
    }
    let (start, id_indent) = rule_start?;

    // Fields are indented at least two spaces beyond the `- ` marker. The
    // canonical layout uses `  - id:` at indent 2 and fields at indent 4,
    // giving `field_indent = id_indent + 2`. Confirm by probing the next
    // non-blank line.
    let mut field_indent = id_indent + 2;
    for line in &lines[start + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading > id_indent {
            field_indent = leading;
        }
        break;
    }

    // End is the first subsequent line that opens a new rule (another
    // `- id:` at the same indent) or a top-level key. The rule block itself
    // includes every line whose indent is >= id_indent and does not start a
    // new rule.
    let mut end = lines.len();
    for (offset, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading <= id_indent && !line.starts_with(' ') && !line.trim_start().starts_with('-') {
            end = offset;
            break;
        }
        if leading == id_indent && line.trim_start().starts_with("- ") {
            end = offset;
            break;
        }
    }

    Some((start, end, field_indent))
}

fn mut_block_end(lines: &[String], start: usize) -> usize {
    // Locate the end again using the mutated lines; cheap for our file sizes.
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let start_re = Regex::id_line();
    let id_indent = refs[start].chars().take_while(|c| *c == ' ').count();
    for (offset, line) in refs.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading <= id_indent && !line.starts_with(' ') && !line.trim_start().starts_with('-') {
            return offset;
        }
        if leading == id_indent && line.trim_start().starts_with("- ") {
            return offset;
        }
        let _ = start_re;
    }
    refs.len()
}

fn replace_field_in_block(
    lines: &mut [String],
    start: usize,
    end: usize,
    field_indent: usize,
    key: &str,
    value: &str,
) -> bool {
    let prefix = format!("{}{key}:", " ".repeat(field_indent));
    for line in lines.iter_mut().take(end).skip(start + 1) {
        let trimmed = line.trim_end_matches(&['\n', '\r'][..]);
        if trimmed.starts_with(&prefix) {
            // Preserve trailing comments that appear after the value.
            let after_colon = &trimmed[prefix.len()..];
            let comment = extract_trailing_comment(after_colon);
            let tail = match comment {
                Some(c) => format!("  {c}"),
                None => String::new(),
            };
            let newline = if line.ends_with('\n') { "\n" } else { "" };
            *line = format!("{prefix} {value}{tail}{newline}");
            return true;
        }
    }
    false
}

fn remove_field_in_block(
    lines: &mut Vec<String>,
    start: usize,
    end: &mut usize,
    field_indent: usize,
    key: &str,
) {
    let prefix = format!("{}{key}:", " ".repeat(field_indent));
    let mut i = start + 1;
    while i < *end {
        if lines[i]
            .trim_end_matches(&['\n', '\r'][..])
            .starts_with(&prefix)
        {
            lines.remove(i);
            *end -= 1;
        } else {
            i += 1;
        }
    }
}

fn insert_field_at_end(
    lines: &mut Vec<String>,
    end: usize,
    field_indent: usize,
    key: &str,
    value: &str,
) {
    // Insert before the block boundary. If the block's last populated line
    // lacks a trailing newline, give it one first so the new entry starts on
    // its own line.
    let insert_at = skip_trailing_blank_lines(lines, end);
    let last = insert_at.saturating_sub(1);
    if last < lines.len() && !lines[last].ends_with('\n') {
        lines[last].push('\n');
    }
    lines.insert(
        insert_at,
        format!("{}{key}: {value}\n", " ".repeat(field_indent)),
    );
}

fn skip_trailing_blank_lines(lines: &[String], end: usize) -> usize {
    let mut i = end;
    while i > 0 && lines[i - 1].trim().is_empty() {
        i -= 1;
    }
    i
}

fn extract_trailing_comment(after_colon: &str) -> Option<&str> {
    // A `#` counts as a comment only when it's whitespace-separated from the
    // value. This prevents us from clipping content like a URL fragment.
    let bytes = after_colon.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return Some(&after_colon[i..]);
        }
    }
    None
}

/// Tiny holder for the lazily-compiled `- id:` regex so the common path
/// avoids re-parsing the pattern per call.
struct Regex;
impl Regex {
    fn id_line() -> &'static regex::Regex {
        static CELL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        CELL.get_or_init(|| {
            regex::Regex::new(r#"^(\s*)- id:\s*['"]?([^'"\s]+)['"]?\s*$"#).expect("valid regex")
        })
    }
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
    // POSIX guarantees a single write(2) to an O_APPEND file is atomic up to
    // PIPE_BUF (4096 bytes on macOS/Linux), so we serialise the line + newline
    // once and issue exactly one write_all. `writeln!` would split into two
    // syscalls and interleave under concurrent `wh apply` calls.
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    f.write_all(line.as_bytes())?;
    f.sync_data().ok();
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

pub fn format_diff(result: &Value) -> String {
    let mut out = String::new();
    let summary = result.get("summary").cloned().unwrap_or(Value::Null);
    let total = summary
        .get("total_candidates")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let deps = summary
        .get("candidate_deps")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let shadow = summary
        .get("would_shadow")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dep_count = summary
        .get("advisory_deprecations")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    out.push_str(&format!(
        "wh review diff: {total} candidate(s) across {deps} dep(s)\n"
    ));
    if shadow > 0 {
        out.push_str(&format!("  {shadow} candidate(s) share an id with an existing approved rule\n"));
    }
    if dep_count > 0 {
        out.push_str(&format!("  {dep_count} approved rule(s) have no corresponding candidate (advisory deprecations)\n"));
    }
    if let Some(per_dep) = result.get("per_dep").and_then(|v| v.as_array()) {
        for entry in per_dep {
            let dep = entry.get("dependency").and_then(|v| v.as_str()).unwrap_or("?");
            let lang = entry.get("language").and_then(|v| v.as_str()).unwrap_or("?");
            let cands = entry
                .get("candidates")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let approved = entry
                .get("approved_existing")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            out.push_str(&format!(
                "  [{lang}] {dep} — {cands} candidate(s), {approved} approved\n"
            ));
        }
    }
    out
}

pub fn format_worklist(result: &Value) -> String {
    let mut out = String::new();
    let total = result.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
    out.push_str(&format!("wh review worklist: {total} entry/entries\n"));
    if let Some(entries) = result.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let lang = entry.get("language").and_then(|v| v.as_str()).unwrap_or("?");
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
    fn validate_transition_forbids_candidate_to_deprecated() {
        let err = validate_transition("candidate", "deprecated").unwrap_err();
        assert!(err.to_string().contains("illegal transition"));
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

    #[test]
    fn validate_reasons_requires_supersede_reason() {
        assert!(validate_reasons(Transition::Supersede, None, Some("other.id")).is_err());
    }

    const SAMPLE_YAML: &str = r#"source:
  name: demo
  docs_url: https://example.com
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

# top-of-file comment stays
rules:
  - id: demo.rule
    # rationale comment stays
    severity: must
    confidence: high
    category: default
    description: Test rule
    source_url: https://example.com/rule
    approved: false
    status: candidate
    proposed_at: "2026-04-14T00:00:00Z"
    signals:
      - id: s1
        strategy: pattern
        description: signal
        weight: required
        match: 'TODO'
    golden_examples:
      - code: ""
        verdict: pass
        reason: placeholder
"#;

    #[test]
    fn apply_rule_updates_preserves_comments_and_blank_lines() {
        let updates = vec![
            ("status", "approved".to_string()),
            ("approved", "true".to_string()),
            ("approved_at", "\"2026-04-14T00:00:01Z\"".to_string()),
        ];
        let out = apply_rule_updates(SAMPLE_YAML, "demo.rule", &updates, &[]).unwrap();
        assert!(
            out.contains("# top-of-file comment stays"),
            "top comment dropped:\n{out}"
        );
        assert!(
            out.contains("# rationale comment stays"),
            "in-block comment dropped:\n{out}"
        );
        assert!(out.contains("status: approved"));
        assert!(out.contains("approved: true"));
        assert!(out.contains("approved_at: \"2026-04-14T00:00:01Z\""));
    }

    #[test]
    fn apply_rule_updates_appends_missing_fields() {
        let updates = vec![
            ("status", "denied".to_string()),
            ("approved", "false".to_string()),
            ("denied_reason", "\"not applicable\"".to_string()),
        ];
        let out = apply_rule_updates(SAMPLE_YAML, "demo.rule", &updates, &[]).unwrap();
        assert!(
            out.contains("denied_reason: \"not applicable\""),
            "denied_reason not inserted:\n{out}"
        );
        assert_eq!(out.matches("- id: demo.rule").count(), 1);
    }

    #[test]
    fn format_diff_renders_summary_and_per_dep_lines() {
        let v = json!({
            "summary": {
                "total_candidates": 2,
                "candidate_deps": 1,
                "would_shadow": 1,
                "advisory_deprecations": 0,
            },
            "per_dep": [{
                "dependency": "fastapi",
                "language": "python",
                "candidates": [{"id": "fastapi.a"}, {"id": "fastapi.b"}],
                "approved_existing": 1,
            }],
        });
        let s = format_diff(&v);
        assert!(s.contains("2 candidate(s) across 1 dep(s)"), "{s}");
        assert!(s.contains("fastapi"), "{s}");
        assert!(s.contains("share an id"), "{s}");
    }

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
            "next_command": "wh propose import <bundle>",
        });
        let s = format_worklist(&v);
        assert!(s.contains("ready_now"), "{s}");
        assert!(s.contains("fastapi"), "{s}");
        assert!(s.contains("Read the linked source"), "{s}");
        assert!(s.contains("wh propose import"), "{s}");
    }

    #[test]
    fn quote_yaml_string_quotes_reserved_values() {
        assert_eq!(quote_yaml_string("hello"), "hello");
        assert_eq!(quote_yaml_string("true"), "\"true\"");
        // ISO timestamps contain `:` which is a YAML flow indicator, so we
        // always quote them to sidestep plain-scalar ambiguity.
        assert_eq!(
            quote_yaml_string("2026-04-14T00:00:00Z"),
            "\"2026-04-14T00:00:00Z\""
        );
        assert!(quote_yaml_string("contains: colon").starts_with('"'));
        assert!(quote_yaml_string("# leading hash").starts_with('"'));
    }
}
