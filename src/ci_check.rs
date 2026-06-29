use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use crate::state::StateManager;
use crate::status;

pub fn ci_check(project_dir: &Path, check_drift: bool, changed_only: bool) -> Result<Value> {
    let start = Instant::now();
    let effective_drift = check_drift || changed_only;

    let status_result = status::compute_status(project_dir, effective_drift, changed_only)?;

    if status_result.get("status").and_then(|v| v.as_str()) == Some("not_initialized") {
        return Ok(serde_json::json!({
            "freshness_status": "not_initialized",
            "changed_sources_count": 0,
            "recommended_rules_count": 0,
            "requires_review": false,
            "score": 0,
            "label": "Not Initialized",
            "message": "Whetstone not initialized in this project.",
            "elapsed_seconds": (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        }));
    }

    let label = status_result
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let score = status_result
        .get("score")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dims = status_result
        .get("dimensions")
        .cloned()
        .unwrap_or(Value::Null);
    let recommendations = status_result
        .get("recommendations")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    let pending_updates = dims
        .get("pending_updates")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let freshness_status = match label {
        "Healthy" => "healthy",
        "Needs Review" => "needs_review",
        "Stale" => "stale",
        "No Rules" => "no_rules",
        _ => "unknown",
    };

    let elapsed = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let rec_count = recommendations.as_array().map(|a| a.len()).unwrap_or(0);

    // Offline content-hash drift: the docs changed since the rules were authored.
    // This needs no network — it compares each rule's authored `content_hash`
    // against the hash of the currently-cached resolved documentation.
    let content_drift = if effective_drift {
        compute_content_drift(project_dir)
    } else {
        Vec::new()
    };
    // Content drift is a review trigger: force at least "needs_review" so the CI
    // gate (default --fail-on needs_review) flags it.
    let freshness_status = if !content_drift.is_empty() && freshness_status == "healthy" {
        "needs_review"
    } else {
        freshness_status
    };
    let requires_review =
        matches!(freshness_status, "stale" | "needs_review") || !content_drift.is_empty();

    Ok(serde_json::json!({
        "freshness_status": freshness_status,
        "changed_sources_count": pending_updates,
        "recommended_rules_count": rec_count,
        "requires_review": requires_review,
        "score": score,
        "label": label,
        "dimensions": dims,
        "recommendations": recommendations,
        "content_drift": content_drift,
        "content_drift_count": content_drift.len(),
        "elapsed_seconds": elapsed,
        "next_command": status_result.get("next_command"),
    }))
}

/// Compare each approved rule file's authored `source.content_hash` against the
/// hash of the currently-cached resolved documentation for that source. A
/// mismatch means the docs changed since the rule was authored — the rule may
/// be stale. Offline: reads only the persisted source cache written by the last
/// `wh init` / `wh reinit`. Rules whose `content_hash` is a placeholder (e.g.
/// dogfood / manually-authored) are skipped — there is nothing to compare.
fn compute_content_drift(project_dir: &Path) -> Vec<Value> {
    // Build (language, name) -> current content hash from the source cache.
    let mut sm = StateManager::new(project_dir);
    let mut current: HashMap<(String, String), String> = HashMap::new();
    for entry in sm.cache.all_entries() {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        let lang = entry
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
            current.insert((lang, name), crate::resolve::content_hash(content));
        }
    }
    if current.is_empty() {
        return Vec::new();
    }

    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let rule_dirs = [paths.project_rules_dir.clone(), paths.personal_rules_dir];

    let mut drift = Vec::new();
    for dir in &rule_dirs {
        if !dir.exists() {
            continue;
        }
        let (files, _warnings) = crate::rules::load_rule_files(dir);
        for lf in files {
            let src = &lf.rule_file.source;
            let Some(authored) = src.content_hash.as_deref() else {
                continue;
            };
            // Only compare real resolved hashes — skip placeholders we can't verify.
            if !authored.starts_with("sha256:")
                || authored.contains("dogfood")
                || authored.contains("manual")
                || authored.contains("placeholder")
            {
                continue;
            }
            let name = src.name.to_lowercase();
            let lang = lf.language.clone().unwrap_or_default().to_lowercase();
            // Prefer an exact (language, name) match; fall back to name-only so a
            // version/language mismatch in the cache key still surfaces drift.
            let cur = current
                .get(&(lang.clone(), name.clone()))
                .or_else(|| current.iter().find(|((_, n), _)| *n == name).map(|(_, h)| h));
            if let Some(cur_hash) = cur {
                if cur_hash != authored {
                    drift.push(json!({
                        "dependency": src.name,
                        "language": lf.language,
                        "authored_content_hash": authored,
                        "current_content_hash": cur_hash,
                        "rule_file": lf.file_path,
                        "message": format!(
                            "Documentation for `{}` changed since its rules were authored — review and re-extract (`wh reinit` then `wh extract`); the cached docs no longer match the rule's content_hash.",
                            src.name
                        ),
                    }));
                }
            }
        }
    }
    drift
}

pub fn format_pr_comment(result: &Value) -> String {
    let marker = "<!-- whetstone-ci-check -->";
    let status_emoji = match result
        .get("freshness_status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
    {
        "healthy" => "OK",
        "needs_review" => "!!",
        "stale" => "XX",
        "no_rules" | "not_initialized" => "--",
        _ => "??",
    };

    let label = result
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let score = result.get("score").and_then(|v| v.as_i64()).unwrap_or(0);

    let mut lines = vec![
        marker.to_string(),
        "## Whetstone Status".to_string(),
        String::new(),
        format!("**[{}] {}** (score: {}/100)", status_emoji, label, score),
        String::new(),
    ];

    if let Some(dims) = result.get("dimensions").and_then(|v| v.as_object()) {
        lines.push("| Metric | Value |".to_string());
        lines.push("|--------|-------|".to_string());
        if let Some(freshness) = dims.get("freshness_days").and_then(|v| v.as_f64()) {
            lines.push(format!("| Freshness | {:.0} days |", freshness));
        }
        lines.push(format!(
            "| Rules | {} approved |",
            dims.get("rules_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        ));
        lines.push(format!(
            "| High confidence | {:.0}% |",
            dims.get("high_confidence_ratio")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        ));
        lines.push(format!(
            "| Deterministic coverage | {:.0}% |",
            dims.get("deterministic_coverage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        ));
        lines.push(format!(
            "| Pending updates | {} deps |",
            dims.get("pending_updates")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        ));
        lines.push(String::new());
    }

    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        lines.push(format!("**Next:** `{}`", next));
        lines.push(String::new());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn setup(tmp: &Path, authored_hash: &str, cached_content: &str) {
        let state = tmp.join("whetstone").join(".state");
        fs::create_dir_all(&state).unwrap();
        let cache = json!({
            "version": 1,
            "entries": {
                "python:widget:1.0": {
                    "name": "widget",
                    "language": "python",
                    "version": "1.0",
                    "content": cached_content,
                }
            }
        });
        fs::write(
            state.join("source-cache.json"),
            serde_json::to_string(&cache).unwrap(),
        )
        .unwrap();

        let rules = tmp.join("whetstone").join("rules").join("python");
        fs::create_dir_all(&rules).unwrap();
        let rule = format!(
            "source:\n  name: widget\n  version: '1.0'\n  content_hash: '{authored_hash}'\nrules:\n  - id: widget.r\n    severity: should\n    confidence: high\n    category: convention\n    description: x\n    source_url: https://example.com\n    approved: true\n    status: approved\n    signals:\n      - id: s\n        strategy: ast\n        weight: required\n        ast_query: '(function_definition) @match'\n"
        );
        fs::write(rules.join("widget.yaml"), rule).unwrap();
    }

    #[test]
    fn content_drift_detected_when_hash_differs() {
        let tmp = tempfile::tempdir().unwrap();
        // Authored hash deliberately does not match the hash of the cached docs.
        setup(tmp.path(), "sha256:deadbeefdeadbeef", "brand new docs content");
        let drift = compute_content_drift(tmp.path());
        assert_eq!(drift.len(), 1, "{drift:?}");
        assert_eq!(drift[0]["dependency"], "widget");
        assert!(drift[0]["message"]
            .as_str()
            .unwrap()
            .contains("changed since its rules were authored"));
    }

    #[test]
    fn no_drift_when_hash_matches_cached_docs() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "stable docs content";
        let matching = crate::resolve::content_hash(content);
        setup(tmp.path(), &matching, content);
        let drift = compute_content_drift(tmp.path());
        assert!(drift.is_empty(), "{drift:?}");
    }

    #[test]
    fn placeholder_content_hash_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path(), "sha256:dogfood-manual-extraction", "any content");
        let drift = compute_content_drift(tmp.path());
        assert!(drift.is_empty(), "placeholder must be skipped: {drift:?}");
    }

    #[test]
    fn no_cache_means_no_drift() {
        let tmp = tempfile::tempdir().unwrap();
        // Rules but no source cache at all -> nothing to compare against.
        let rules = tmp.path().join("whetstone").join("rules").join("python");
        fs::create_dir_all(&rules).unwrap();
        fs::write(
            rules.join("widget.yaml"),
            "source:\n  name: widget\n  content_hash: 'sha256:abc123'\nrules: []\n",
        )
        .unwrap();
        let drift = compute_content_drift(tmp.path());
        assert!(drift.is_empty(), "{drift:?}");
    }
}
