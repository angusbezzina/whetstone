//! Dependency-scoped extraction worklists.
//!
//! `wh doctor` and `wh refresh` already produce an extraction handoff;
//! the worklist adds a per-dependency view on top: each entry bundles
//! ranked sources, section summaries, quotas derived from config, and
//! a concrete next-step hint for the agent. The goal is "work one dep
//! at a time" without the agent re-deriving priority from a flat list.
//!
//! The worklist is persisted as the `worklist` key inside
//! `whetstone/.state/extraction-handoff.json` (fully additive — older
//! readers ignore the unknown key) and also exposed standalone through
//! the `review worklist` subcommand, which filters by dep name or
//! language.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::WhetstoneConfig;

/// Priority bucket for a worklist entry. Callers order by (ready_now,
/// resolved_low, pending, failed) then by configured preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    ReadyNow,
    ResolvedLow,
    Pending,
    Failed,
    Skipped,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::ReadyNow => "ready_now",
            Priority::ResolvedLow => "resolved_low",
            Priority::Pending => "pending",
            Priority::Failed => "failed",
            Priority::Skipped => "skipped",
        }
    }

    fn order_key(&self) -> u8 {
        match self {
            Priority::ReadyNow => 0,
            Priority::ResolvedLow => 1,
            Priority::Pending => 2,
            Priority::Failed => 3,
            Priority::Skipped => 4,
        }
    }
}

/// Build a per-dependency worklist from the assembled doctor result.
/// `doctor_result` is the JSON returned by `doctor::doctor`, already
/// carrying `extraction_context`, `extraction_subsets`, and
/// `resolution_buckets`.
///
/// Returns an array of per-dep JSON objects in priority order.
pub fn build_from_doctor(
    project_dir: &Path,
    doctor_result: &Value,
    existing_rules: &BTreeMap<(String, String), usize>,
) -> Vec<Value> {
    let config = WhetstoneConfig::load(project_dir);

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
    let ready_names = name_set(&subsets, "ready_now");
    let failed_entries = name_reason_map(&subsets, "failed");
    let pending_entries = name_reason_map(&subsets, "pending");

    let preferred_source_kinds: Vec<String> = config.extraction.preferred_source_kinds.clone();
    let recency_cutoff = config
        .extraction
        .recency_window_days
        .map(|d| chrono::Utc::now() - chrono::Duration::days(d as i64));

    let mut entries: Vec<Value> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for source in &sources {
        let name = source
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let language = source
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| infer_language(source));

        if !config.extraction_allows(&name) {
            entries.push(build_entry(
                &name,
                &language,
                Priority::Skipped,
                Some("filtered by extraction.include / extraction.exclude"),
                None,
                &[],
                0,
                &config,
                0.0,
            ));
            seen.insert(format!("{language}:{name}"));
            continue;
        }

        let priority = if ready_names.contains(&name) {
            Priority::ReadyNow
        } else {
            Priority::ResolvedLow
        };

        let sections = sections_view(source);
        let existing = existing_rules
            .get(&(language.clone(), name.clone()))
            .copied()
            .unwrap_or(0);

        let score = score_entry(
            source,
            priority,
            &preferred_source_kinds,
            recency_cutoff.as_ref(),
        );

        entries.push(build_entry(
            &name,
            &language,
            priority,
            None,
            Some(source),
            &sections,
            existing,
            &config,
            score,
        ));
        seen.insert(format!("{language}:{name}"));
    }

    // Pending entries: dep detected but source not resolved this run.
    for (name, reason) in pending_entries {
        let key = guess_key(&name);
        if seen.contains(&key) {
            continue;
        }
        if !config.extraction_allows(&name) {
            continue;
        }
        entries.push(build_entry(
            &name,
            "",
            Priority::Pending,
            Some(if reason.is_empty() {
                "source not resolved yet; run `wh doctor --resume` or `wh refresh`"
            } else {
                reason.as_str()
            }),
            None,
            &[],
            0,
            &config,
            0.0,
        ));
    }

    // Failed entries: resolution failure, ops must triage.
    for (name, reason) in failed_entries {
        let key = guess_key(&name);
        if seen.contains(&key) {
            continue;
        }
        entries.push(build_entry(
            &name,
            "",
            Priority::Failed,
            Some(if reason.is_empty() {
                "resolution failed"
            } else {
                reason.as_str()
            }),
            None,
            &[],
            0,
            &config,
            0.0,
        ));
    }

    entries.sort_by(|a, b| {
        let pa = a
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pb = b
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ord_a = priority_from_str(pa).order_key();
        let ord_b = priority_from_str(pb).order_key();
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ord_a.cmp(&ord_b).then_with(|| {
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    entries
}

/// Lightweight loader for `wh review worklist`: reads the existing
/// extraction-handoff artifact and returns the `worklist` array (or
/// a helpful error if the artifact has not been generated yet).
pub fn load(project_dir: &Path) -> Result<Value> {
    let path = project_dir
        .join("whetstone")
        .join(".state")
        .join("extraction-handoff.json");
    if !path.exists() {
        return Err(anyhow!(
            "extraction-handoff.json not found. Run `wh doctor` or `wh refresh` first."
        ));
    }
    let text = std::fs::read_to_string(&path)?;
    let handoff: Value = serde_json::from_str(&text)?;
    Ok(handoff)
}

/// Filter a worklist by dep name and/or language. Empty filter = pass-through.
pub fn filter(worklist: &[Value], dep: Option<&str>, lang: Option<&str>) -> Vec<Value> {
    worklist
        .iter()
        .filter(|entry| {
            if let Some(d) = dep {
                let n = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if !n.eq_ignore_ascii_case(d) {
                    return false;
                }
            }
            if let Some(l) = lang {
                let el = entry.get("language").and_then(|v| v.as_str()).unwrap_or("");
                if !el.eq_ignore_ascii_case(l) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

// ── Internals ──

#[allow(clippy::too_many_arguments)]
fn build_entry(
    name: &str,
    language: &str,
    priority: Priority,
    reason: Option<&str>,
    source: Option<&Value>,
    sections: &[Value],
    existing_rules: usize,
    config: &WhetstoneConfig,
    score: f64,
) -> Value {
    let max_rules_per_dep = config
        .extraction
        .max_rules_per_dep
        .unwrap_or(5);
    let remaining_quota = max_rules_per_dep.saturating_sub(existing_rules as u32);
    let next_step = next_step_hint(priority, remaining_quota, source);

    let mut entry = json!({
        "name": name,
        "language": language,
        "priority": priority.as_str(),
        "score": score,
        "sections": sections,
        "existing_rules": existing_rules,
        "quota": {
            "max_rules_per_dep": max_rules_per_dep,
            "remaining": remaining_quota,
        },
        "next_step": next_step,
    });

    if let Some(src) = source {
        if let Some(v) = src.get("version") {
            entry["version"] = v.clone();
        }
        if let Some(v) = src.get("source_type") {
            entry["source_type"] = v.clone();
        }
        if let Some(v) = src
            .get("docs_url")
            .or_else(|| src.get("source_url"))
        {
            entry["source_url"] = v.clone();
        }
        if let Some(v) = src.get("content_hash") {
            entry["content_hash"] = v.clone();
        }
        if let Some(v) = src.get("registry") {
            entry["registry"] = v.clone();
        }
        if let Some(f) = src.get("freshness") {
            entry["freshness"] = f.clone();
        }
    }

    if !config.extraction.allowed_categories.is_empty() {
        entry["allowed_categories"] =
            json!(config.extraction.allowed_categories.clone());
    }
    if let Some(ref mc) = config.extraction.min_confidence {
        entry["min_confidence"] = Value::String(mc.clone());
    }
    if !config.extraction.preferred_source_kinds.is_empty() {
        entry["preferred_source_kinds"] =
            json!(config.extraction.preferred_source_kinds.clone());
    }

    if let Some(r) = reason {
        entry["reason"] = Value::String(r.to_string());
    }

    entry
}

fn sections_view(source: &Value) -> Vec<Value> {
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
                    let mut obj = serde_json::Map::new();
                    if let Some(v) = s.get("type") {
                        obj.insert("type".into(), v.clone());
                    }
                    if let Some(v) = s.get("url") {
                        obj.insert("url".into(), v.clone());
                    }
                    if let Some(v) = s.get("versions_covered") {
                        obj.insert("versions_covered".into(), v.clone());
                    }
                    if let Some(v) = s.get("source_kind") {
                        obj.insert("source_kind".into(), v.clone());
                    }
                    if let Some(v) = s.get("published_at") {
                        obj.insert("published_at".into(), v.clone());
                    }
                    obj.insert("bytes".into(), Value::from(bytes));
                    Value::Object(obj)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn score_entry(
    source: &Value,
    priority: Priority,
    preferred_kinds: &[String],
    recency_cutoff: Option<&chrono::DateTime<chrono::Utc>>,
) -> f64 {
    let mut score = match priority {
        Priority::ReadyNow => 100.0,
        Priority::ResolvedLow => 40.0,
        Priority::Pending => 5.0,
        Priority::Failed => 1.0,
        Priority::Skipped => 0.0,
    };

    let source_type = source
        .get("source_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    score += match source_type {
        "llms_full_txt" => 25.0,
        "llms_txt" => 15.0,
        "readme" => 10.0,
        "html_converted" => 5.0,
        _ => 0.0,
    };

    if let Some(sections) = source.get("sections").and_then(|v| v.as_array()) {
        for section in sections {
            let kind = section
                .get("source_kind")
                .or_else(|| section.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(rank) = preferred_kinds.iter().position(|k| k == kind) {
                score += ((preferred_kinds.len() - rank) as f64) * 2.0;
            }
            if let Some(cutoff) = recency_cutoff {
                if let Some(ts) = section
                    .get("published_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                {
                    if ts.with_timezone(&chrono::Utc) >= *cutoff {
                        score += 5.0;
                    }
                }
            }
        }
    }

    score
}

fn next_step_hint(priority: Priority, remaining_quota: u32, source: Option<&Value>) -> String {
    match priority {
        Priority::ReadyNow => {
            if remaining_quota == 0 {
                "Quota reached — review existing rules or raise extraction.max_rules_per_dep".into()
            } else {
                format!(
                    "Read the linked source, propose up to {remaining_quota} rule(s), then `wh propose import <bundle>`"
                )
            }
        }
        Priority::ResolvedLow => {
            let hint = source
                .and_then(|s| s.get("source_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("low-confidence source");
            format!(
                "Source is `{hint}` — proceed with extra caution; prefer migration/breaking-change rules with direct citations"
            )
        }
        Priority::Pending => {
            "Resolve with `wh set-sources --deps=<name>` or `wh doctor --resume`".into()
        }
        Priority::Failed => {
            "Add a manual entry under `sources.custom` in whetstone.yaml and re-run".into()
        }
        Priority::Skipped => {
            "Dependency filtered out by config — adjust extraction.include / exclude if this was unintended".into()
        }
    }
}

fn name_set(subsets: &Value, key: &str) -> BTreeSet<String> {
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

fn name_reason_map(subsets: &Value, key: &str) -> Vec<(String, String)> {
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

fn infer_language(source: &Value) -> String {
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

fn guess_key(name: &str) -> String {
    // Pending/failed entries don't carry language; build a best-effort
    // key for deduplication.
    format!(":{name}")
}

fn priority_from_str(s: &str) -> Priority {
    match s {
        "ready_now" => Priority::ReadyNow,
        "resolved_low" => Priority::ResolvedLow,
        "pending" => Priority::Pending,
        "failed" => Priority::Failed,
        _ => Priority::Skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_ordering_is_stable() {
        let entries = vec![
            json!({"name": "b", "priority": "resolved_low", "score": 10.0}),
            json!({"name": "a", "priority": "ready_now", "score": 5.0}),
            json!({"name": "c", "priority": "ready_now", "score": 15.0}),
        ];
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| {
            let pa = a.get("priority").and_then(|v| v.as_str()).unwrap_or("");
            let pb = b.get("priority").and_then(|v| v.as_str()).unwrap_or("");
            let ord_a = priority_from_str(pa).order_key();
            let ord_b = priority_from_str(pb).order_key();
            let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            ord_a.cmp(&ord_b).then(
                sb.partial_cmp(&sa)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let names: Vec<&str> = sorted
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[test]
    fn filter_respects_dep_and_lang() {
        let wl = vec![
            json!({"name": "fastapi", "language": "python"}),
            json!({"name": "react", "language": "typescript"}),
            json!({"name": "FastAPI", "language": "python"}),
        ];
        assert_eq!(filter(&wl, Some("fastapi"), None).len(), 2);
        assert_eq!(filter(&wl, None, Some("python")).len(), 2);
        assert_eq!(filter(&wl, Some("react"), Some("python")).len(), 0);
    }

    #[test]
    fn next_step_respects_quota_exhaustion() {
        let hint = next_step_hint(Priority::ReadyNow, 0, None);
        assert!(hint.contains("Quota reached"));
    }
}
