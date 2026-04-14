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
            "reason": "Source not yet resolved; run wh doctor --resume or wh refresh.",
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

    let handoff = json!({
        "version": 1,
        "generated_at": now_iso(),
        "trigger": trigger,
        "project_dir": project_dir.display().to_string(),
        "languages": languages,
        "candidates": candidates,
        "skipped": skipped,
        "next_action": "Apply extraction prompt to each candidate; approve or deny; then wh validate && wh context && wh tests",
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

    let drift_count = changed_entries.len() as i64;

    let diff = json!({
        "version": 1,
        "generated_at": now_iso(),
        "project_dir": project_dir.display().to_string(),
        "drift_count": drift_count,
        "changed": changed_entries,
        "unchanged_with_stale_cache": Value::Array(Vec::new()),
        "removed": removed,
        "failed": failed,
        "next_action": if drift_count == 0 {
            "No drift detected. Rules are current.".to_string()
        } else {
            "For each changed dep, re-read its new content and propose: new rules, modified rules, rules to deprecate (status: deprecated).".to_string()
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

fn load_approved_counts(project_dir: &Path) -> BTreeMap<(String, String), usize> {
    let mut counts = BTreeMap::new();
    for (lang, dep, _rid) in load_approved_index(project_dir) {
        *counts.entry((lang, dep)).or_insert(0) += 1;
    }
    counts
}
