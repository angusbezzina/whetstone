//! Durable handoff artifacts between the Whetstone binary and the agent.
//!
//! See `references/handoff-schema.md` for the on-disk JSON contract. These
//! files let the agent resume extraction or refresh work after a crash and
//! give the user a reviewable audit trail.

use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::state::atomic_write;

const HANDOFF_FILE: &str = "extraction-handoff.json";
const REFRESH_DIFF_FILE: &str = "refresh-diff.json";

fn state_dir(project_dir: &Path) -> std::path::PathBuf {
    project_dir.join("whetstone").join(".state")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Build and persist the extraction handoff from a doctor-style result.
///
/// `trigger` is `"doctor"` or `"refresh"`. The doctor result is expected to
/// contain `extraction_context`, `extraction_subsets`, and `resolution_buckets`
/// as built by `doctor::doctor`.
pub fn write_extraction_handoff(
    project_dir: &Path,
    trigger: &str,
    doctor_result: &Value,
) -> Result<std::path::PathBuf> {
    let dir = state_dir(project_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(HANDOFF_FILE);

    let languages = doctor_result
        .get("summary")
        .and_then(|s| s.get("languages"))
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));

    let sources: Vec<Value> = doctor_result
        .get("extraction_context")
        .and_then(|v| v.get("sources"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let subsets = doctor_result
        .get("extraction_subsets")
        .cloned()
        .unwrap_or(Value::Null);
    let ready_names = extract_name_set(&subsets, "ready_now");
    let pending_names = extract_name_set(&subsets, "pending");
    let failed_subset = extract_name_map(&subsets, "failed");
    let existing_rules = load_approved_counts(project_dir);

    let mut candidates: Vec<Value> = Vec::new();
    for source in &sources {
        let name = source
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let language = source
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| infer_language_from_sections(source));

        let priority = if ready_names.contains(&name) {
            "ready_now"
        } else {
            "resolved_low"
        };

        candidates.push(json!({
            "name": name,
            "language": language,
            "version": source.get("version").cloned().unwrap_or(Value::Null),
            "source_type": source.get("source_type").cloned().unwrap_or(Value::Null),
            "source_url": source.get("docs_url")
                .or_else(|| source.get("source_url"))
                .cloned()
                .unwrap_or(Value::Null),
            "content_hash": source.get("content_hash").cloned().unwrap_or(Value::Null),
            "sections": summarize_sections(source),
            "existing_rules": existing_rules.get(&(language.clone(), name.clone())).copied().unwrap_or(0),
            "priority": priority,
        }));
    }

    // Append pending (not resolved this run) and failed (resolution error).
    for name in &pending_names {
        candidates.push(json!({
            "name": name,
            "priority": "pending",
            "reason": "Source not yet resolved; run wh init --resume or wh reinit.",
        }));
    }
    for (name, reason) in &failed_subset {
        candidates.push(json!({
            "name": name,
            "priority": "failed",
            "reason": reason,
        }));
    }

    let skipped: Vec<Value> = doctor_result
        .get("resolution_buckets")
        .and_then(|b| b.get("cached"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let name = c.get("name").cloned().unwrap_or(Value::Null);
            json!({
                "name": name,
                "reason": "Already cached and unchanged; no re-resolution needed.",
            })
        })
        .collect();

    let worklist = crate::worklist::build_from_doctor(project_dir, doctor_result, &existing_rules);

    let handoff = json!({
        "version": 1,
        "generated_at": now_iso(),
        "trigger": trigger,
        "project_dir": project_dir.display().to_string(),
        "languages": languages,
        "candidates": candidates,
        "skipped": skipped,
        "worklist": worklist,
        "next_action": "Work the worklist top-down; for each ready_now dep, produce a proposal bundle and run `wh propose import`.",
    });

    atomic_write(&path, &handoff);
    Ok(path)
}

/// Build and persist a refresh diff based on a doctor result invoked with
/// `changed_only=true, refresh=true`.
pub fn write_refresh_diff(
    project_dir: &Path,
    doctor_result: &Value,
) -> Result<(std::path::PathBuf, i64)> {
    let dir = state_dir(project_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(REFRESH_DIFF_FILE);

    let sources: Vec<Value> = doctor_result
        .get("extraction_context")
        .and_then(|v| v.get("sources"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let scan = doctor_result.get("scan").cloned().unwrap_or(Value::Null);
    let inventory_diff = scan.get("inventory_diff").cloned().unwrap_or(Value::Null);

    let added: BTreeSet<String> = inventory_keys(&inventory_diff, "added");
    let changed_keys: BTreeSet<String> = inventory_keys(&inventory_diff, "changed");
    let removed_keys: BTreeSet<String> = inventory_keys(&inventory_diff, "removed");

    let all_rules = load_approved_index(project_dir);

    let mut changed_entries: Vec<Value> = Vec::new();
    for source in &sources {
        let name = source
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let language = source
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| infer_language_from_sections(source));

        let key = format!("{language}:{name}");
        let is_changed = added.contains(&key) || changed_keys.contains(&key);
        if !is_changed {
            continue;
        }

        let affected: Vec<String> = all_rules
            .iter()
            .filter(|(lang, dep, _rid)| lang.as_str() == language && dep.as_str() == name)
            .map(|(_lang, _dep, rid)| rid.clone())
            .collect();

        let mut urls = serde_json::Map::new();
        if let Some(sections) = source.get("sections").and_then(|v| v.as_array()) {
            for s in sections {
                if let (Some(stype), Some(url)) = (
                    s.get("type").and_then(|v| v.as_str()),
                    s.get("url").and_then(|v| v.as_str()),
                ) {
                    urls.insert(stype.to_string(), Value::String(url.to_string()));
                }
            }
        }

        changed_entries.push(json!({
            "name": name,
            "language": language,
            "previous_version": Value::Null,
            "current_version": source.get("version").cloned().unwrap_or(Value::Null),
            "previous_content_hash": Value::Null,
            "current_content_hash": source.get("content_hash").cloned().unwrap_or(Value::Null),
            "changed_sections": section_types(source),
            "affected_rule_ids": affected,
            "source_urls": Value::Object(urls),
        }));
    }

    let removed: Vec<Value> = removed_keys
        .iter()
        .map(|k| {
            let (lang, name) = split_key(k);
            json!({
                "name": name,
                "language": lang,
                "reason": "dropped from manifest",
            })
        })
        .collect();

    let failed: Vec<Value> = doctor_result
        .get("resolution_buckets")
        .and_then(|b| b.get("failed"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            json!({
                "name": e.get("name").cloned().unwrap_or(Value::Null),
                "error": e.get("error").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    // Epic 3E · nuh: also flag deps whose manifest version didn't change but
    // whose resolved doc content has. Because reinit invokes doctor with
    // refresh=true, every dep's content_hash is re-fetched — the comparison
    // is free. Each matched dep is added to `changed` with drift_type to
    // disambiguate from manifest-version drift.
    let rule_meta_for_content = load_approved_rule_meta(project_dir);
    let existing_keys: BTreeSet<String> = changed_entries
        .iter()
        .filter_map(|e| {
            let lang = e.get("language").and_then(|v| v.as_str())?;
            let name = e.get("name").and_then(|v| v.as_str())?;
            Some(format!("{lang}:{name}"))
        })
        .collect();

    for source in &sources {
        let name = source
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let language = source
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| infer_language_from_sections(source));
        let key = format!("{language}:{name}");
        if existing_keys.contains(&key) {
            continue;
        }
        let current_hash = source
            .get("content_hash")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if current_hash.is_empty() {
            continue;
        }
        // Any approved rule with a stored hash differing from current_hash?
        let content_drifted = rule_meta_for_content.iter().any(|m| {
            m.dep == name
                && m.language == language
                && m.stored_content_hash
                    .as_deref()
                    .map(|h| h != current_hash)
                    .unwrap_or(false)
        });
        if !content_drifted {
            continue;
        }
        let affected: Vec<String> = rule_meta_for_content
            .iter()
            .filter(|m| m.dep == name && m.language == language)
            .map(|m| m.rule_id.clone())
            .collect();
        let mut urls = serde_json::Map::new();
        if let Some(sections) = source.get("sections").and_then(|v| v.as_array()) {
            for s in sections {
                if let (Some(stype), Some(url)) = (
                    s.get("type").and_then(|v| v.as_str()),
                    s.get("url").and_then(|v| v.as_str()),
                ) {
                    urls.insert(stype.to_string(), Value::String(url.to_string()));
                }
            }
        }
        changed_entries.push(json!({
            "name": name,
            "language": language,
            "previous_version": source.get("version").cloned().unwrap_or(Value::Null),
            "current_version": source.get("version").cloned().unwrap_or(Value::Null),
            "previous_content_hash": Value::Null,
            "current_content_hash": source.get("content_hash").cloned().unwrap_or(Value::Null),
            "changed_sections": section_types(source),
            "affected_rule_ids": affected,
            "source_urls": Value::Object(urls),
            "drift_type": "content_hash",
        }));
    }

    let drift_count = changed_entries.len() as i64;

    // Epic 3E · awj: emit per-rule re-extraction candidates for every approved
    // rule citing a drifted dep. Agents read this to know which rules to
    // re-judge rather than guessing from a flat dep list.
    let rule_meta = load_approved_rule_meta(project_dir);
    let mut re_extraction_candidates: Vec<Value> = Vec::new();
    for entry in &changed_entries {
        let entry_name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let entry_lang = entry
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let current_hash = entry
            .get("current_content_hash")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        for meta in &rule_meta {
            if meta.dep != entry_name || meta.language != entry_lang {
                continue;
            }
            let mut drift_types: Vec<&str> = vec!["version"];
            if let Some(stored) = &meta.stored_content_hash {
                if !current_hash.is_empty() && stored != current_hash {
                    drift_types.push("content_hash");
                }
            }
            re_extraction_candidates.push(json!({
                "rule_id": meta.rule_id,
                "dep": meta.dep,
                "language": meta.language,
                "current_severity": meta.severity,
                "current_source_url": meta.source_url,
                "stored_content_hash": meta.stored_content_hash,
                "latest_content_hash": entry.get("current_content_hash"),
                "drift_types": drift_types,
            }));
        }
    }

    // Epic 3E · jrs: canned re-extraction prompt the agent can act on directly.
    // Keep it terse — the agent knows the workflow; this just names the subset.
    let extraction_prompt = if re_extraction_candidates.is_empty() {
        None
    } else {
        let ids: Vec<String> = re_extraction_candidates
            .iter()
            .filter_map(|c| c.get("rule_id").and_then(|v| v.as_str()).map(String::from))
            .collect();
        Some(format!(
            "Drift detected on {} rule(s): {}. \
             For each rule, re-read the current docs at its `current_source_url` (see this file), \
             decide whether to keep / edit severity via `wh rule edit` / delete / re-author via \
             `wh extract submit <bundle>`. Then run `wh actions` to regenerate outputs.",
            ids.len(),
            ids.join(", ")
        ))
    };

    let diff = json!({
        "version": 1,
        "generated_at": now_iso(),
        "project_dir": project_dir.display().to_string(),
        "drift_count": drift_count,
        "changed": changed_entries,
        "unchanged_with_stale_cache": Value::Array(Vec::new()),
        "removed": removed,
        "failed": failed,
        "re_extraction_candidates": re_extraction_candidates,
        "extraction_prompt": extraction_prompt,
        "next_action": if drift_count == 0 {
            "No drift detected. Rules are current.".to_string()
        } else {
            "Read re_extraction_candidates, then for each entry decide: keep / `wh rule edit` severity / delete rule YAML / `wh extract submit <bundle>` a re-authored version. Finish with `wh actions`.".to_string()
        },
    });

    atomic_write(&path, &diff);
    Ok((path, drift_count))
}

// ── Helpers ──

fn extract_name_set(subsets: &Value, key: &str) -> BTreeSet<String> {
    subsets
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_name_map(subsets: &Value, key: &str) -> Vec<(String, String)> {
    subsets
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let name = e.get("name").and_then(|v| v.as_str())?.to_string();
                    let reason = e
                        .get("error")
                        .or_else(|| e.get("reason"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some((name, reason))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn summarize_sections(source: &Value) -> Vec<Value> {
    source
        .get("sections")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    let bytes = s
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|c| c.len())
                        .unwrap_or(0);
                    let mut out = serde_json::Map::new();
                    if let Some(t) = s.get("type") {
                        out.insert("type".to_string(), t.clone());
                    }
                    if let Some(u) = s.get("url") {
                        out.insert("url".to_string(), u.clone());
                    }
                    if let Some(v) = s.get("versions_covered") {
                        out.insert("versions_covered".to_string(), v.clone());
                    }
                    out.insert("bytes".to_string(), Value::from(bytes));
                    Value::Object(out)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn section_types(source: &Value) -> Vec<Value> {
    source
        .get("sections")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.get("type").cloned()).collect())
        .unwrap_or_default()
}

fn infer_language_from_sections(source: &Value) -> String {
    source
        .get("registry")
        .and_then(|v| v.as_str())
        .map(|r| match r {
            "pypi" => "python",
            "npm" => "typescript",
            "crates_io" => "rust",
            _ => "generic",
        })
        .unwrap_or("generic")
        .to_string()
}

fn inventory_keys(inventory_diff: &Value, key: &str) -> BTreeSet<String> {
    inventory_diff
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn split_key(key: &str) -> (String, String) {
    if let Some((lang, name)) = key.split_once(':') {
        (lang.to_string(), name.to_string())
    } else {
        (String::new(), key.to_string())
    }
}

/// Returns `(language, dep_name, rule_id)` tuples for every approved rule.
fn load_approved_index(project_dir: &Path) -> Vec<(String, String, String)> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _) = crate::rules::load_rule_files(&rules_dir);
    let mut out = Vec::new();
    for lrf in &files {
        let language = lrf.language.clone().unwrap_or_default();
        let source_name = lrf.rule_file.source.name.clone();
        for r in &lrf.rule_file.rules {
            if r.approved {
                out.push((language.clone(), source_name.clone(), r.id.clone()));
            }
        }
    }
    out
}

/// Richer variant that carries enough per-rule metadata to emit
/// `re_extraction_candidates` entries in refresh-diff.json (Epic 3E awj).
#[derive(Debug, Clone)]
struct ApprovedRuleMeta {
    language: String,
    dep: String,
    rule_id: String,
    severity: Option<String>,
    source_url: Option<String>,
    stored_content_hash: Option<String>,
}

fn load_approved_rule_meta(project_dir: &Path) -> Vec<ApprovedRuleMeta> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _) = crate::rules::load_rule_files(&rules_dir);
    let mut out = Vec::new();
    for lrf in &files {
        let language = lrf.language.clone().unwrap_or_default();
        let dep = lrf.rule_file.source.name.clone();
        let stored_hash = lrf.rule_file.source.content_hash.clone();
        for r in &lrf.rule_file.rules {
            if r.approved {
                out.push(ApprovedRuleMeta {
                    language: language.clone(),
                    dep: dep.clone(),
                    rule_id: r.id.clone(),
                    severity: r.severity.clone(),
                    source_url: r.source_url.clone(),
                    stored_content_hash: stored_hash.clone(),
                });
            }
        }
    }
    out
}

/// Count *live* rules per dep — approved + candidate together. This is the
/// number that counts against the per-dep quota from the agent's point of
/// view, since any additional proposals need to fit within what's left.
fn load_approved_counts(project_dir: &Path) -> BTreeMap<(String, String), usize> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _) = crate::rules::load_rule_files(&rules_dir);
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for lrf in &files {
        let language = lrf.language.clone().unwrap_or_default();
        let source_name = lrf.rule_file.source.name.clone();
        for r in &lrf.rule_file.rules {
            let status = r.status.as_deref().unwrap_or({
                if r.approved {
                    "approved"
                } else {
                    "candidate"
                }
            });
            if matches!(status, "approved" | "candidate") {
                *counts
                    .entry((language.clone(), source_name.clone()))
                    .or_insert(0) += 1;
            }
        }
    }
    counts
}
