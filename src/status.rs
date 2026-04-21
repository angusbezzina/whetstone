use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::detect;
use crate::rules;
use crate::state::StateManager;

struct RecommendationInputs<'a> {
    total_rules: usize,
    freshness_days: Option<f64>,
    last_extraction_date: Option<&'a str>,
    deterministic_coverage: f64,
    drifted_count: usize,
    drift_info: &'a Value,
    ai_signal_count: usize,
    unapproved_count: usize,
}

struct ImpactInputs<'a> {
    total_rules: usize,
    approved_rules: &'a [&'a Value],
    all_rules: &'a [Value],
    dep_names: &'a BTreeSet<String>,
    rule_files: &'a [Value],
    deterministic_coverage: f64,
    drifted_count: usize,
    project_dep_count: usize,
}

pub fn compute_status(
    project_dir: &Path,
    check_dep_drift: bool,
    changed_only: bool,
) -> Result<Value> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let state_dir = project_dir.join("whetstone").join(".state");

    let check_dep_drift = check_dep_drift || changed_only;

    // Use the shared gate so personal-only projects (`wh rule add --personal`
    // without explicit init) aren't treated as "not_initialized".
    let initialized = crate::layers::project_is_initialized(project_dir) || state_dir.exists();
    if !initialized {
        return Ok(serde_json::json!({
            "status": "not_initialized",
            "label": "Not Initialized",
            "message": "No whetstone directory found. Run 'wh init' to get started.",
            "next_command": "wh init",
        }));
    }

    let (rule_files, load_warnings) = load_rule_files(&rules_dir);
    let mut drift_info = Value::Null;

    let changed_only_deps_data = if changed_only {
        detect::detect_deps(project_dir, true, &[], &[], false).ok()
    } else {
        None
    };
    let rule_files = if changed_only {
        drift_info = changed_only_deps_data
            .as_ref()
            .map(extract_drift)
            .unwrap_or(serde_json::json!({"changed": [], "count": 0, "checked": 0}));
        let drifted_names: BTreeSet<String> = drift_info
            .get("changed")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        v.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if drifted_names.is_empty() {
            return Ok(serde_json::json!({
                "status": "ok",
                "label": "Healthy",
                "score": 100,
                "changed_only": true,
                "freshness_label": "Unknown",
                "last_extraction_date": null,
                "dimensions": {"freshness_days": null, "rules_count": 0, "high_confidence_ratio": 0, "deterministic_coverage": 0, "pending_updates": 0},
                "breakdown": {"confidence": {"high": 0, "medium": 0}, "severity": {"must": 0, "should": 0, "may": 0}, "categories": {}, "signals": {"deterministic": 0, "ai": 0, "total": 0}},
                "dependencies_covered": [],
                "drift": {"dependency_changes": [], "documentation_stale": [], "count": 0, "checked": 0},
                "pipeline_state": {},
                "cache_stats": {},
                "extraction_readiness": [],
                "metrics": {"rules_approved": 0, "rules_proposed": 0, "approval_rate": 0, "must_rules": 0, "dependencies_covered": 0, "dependencies_total": 0, "dependency_coverage": 0, "deterministic_coverage": 0, "pending_drift": 0},
                "recommendations": [{"priority": "low", "action": "info", "message": "No dependency drift detected. Everything is current.", "command": null}],
                "warnings": [],
                "next_command": "wh status",
                "message": "No dependency drift detected.",
            }));
        }

        rule_files
            .into_iter()
            .filter(|rf| {
                rf.get("source_name")
                    .and_then(|v| v.as_str())
                    .map(|n| drifted_names.contains(&n.to_lowercase()))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        rule_files
    };

    // Aggregate rule data
    let mut all_rules: Vec<Value> = Vec::new();
    let mut dep_names: BTreeSet<String> = BTreeSet::new();

    for rf in &rule_files {
        if let Some(name) = rf.get("source_name").and_then(|v| v.as_str()) {
            dep_names.insert(name.to_string());
        }
        if let Some(rules) = rf.get("rules").and_then(|v| v.as_array()) {
            all_rules.extend(rules.iter().cloned());
        }
    }

    let approved_rules: Vec<&Value> = all_rules
        .iter()
        .filter(|r| r.get("approved").and_then(|v| v.as_bool()).unwrap_or(false))
        .collect();
    let total_rules = approved_rules.len();
    let unapproved_count = all_rules.len() - total_rules;

    let high_confidence = approved_rules
        .iter()
        .filter(|r| r.get("confidence").and_then(|v| v.as_str()) == Some("high"))
        .count();
    let medium_confidence = approved_rules
        .iter()
        .filter(|r| r.get("confidence").and_then(|v| v.as_str()) == Some("medium"))
        .count();
    let high_confidence_ratio = if total_rules > 0 {
        high_confidence as f64 / total_rules as f64 * 100.0
    } else {
        0.0
    };

    // Signals
    let mut all_signals: Vec<String> = Vec::new();
    for r in &approved_rules {
        if let Some(sigs) = r.get("signals").and_then(|v| v.as_array()) {
            for sig in sigs {
                if let Some(s) = sig.as_str() {
                    all_signals.push(s.to_string());
                }
            }
        }
    }

    let deterministic_count = all_signals
        .iter()
        .filter(|s| matches!(s.as_str(), "ast" | "pattern" | "lint_proxy"))
        .count();
    let ai_count = all_signals.iter().filter(|s| s.as_str() == "ai").count();
    let total_signals = all_signals.len();
    let deterministic_coverage = if total_signals > 0 {
        deterministic_count as f64 / total_signals as f64 * 100.0
    } else {
        0.0
    };

    // Severity
    let must_count = approved_rules
        .iter()
        .filter(|r| r.get("severity").and_then(|v| v.as_str()) == Some("must"))
        .count();
    let should_count = approved_rules
        .iter()
        .filter(|r| r.get("severity").and_then(|v| v.as_str()) == Some("should"))
        .count();
    let may_count = approved_rules
        .iter()
        .filter(|r| r.get("severity").and_then(|v| v.as_str()) == Some("may"))
        .count();

    // Categories
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    for r in &approved_rules {
        let cat = r
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        *categories.entry(cat.to_string()).or_insert(0) += 1;
    }

    // Freshness
    let freshness_days = compute_freshness_days(&rule_files);
    let freshness_label = compute_freshness_label(freshness_days);
    let last_extraction_date = compute_last_extraction_date(&rule_files);

    // Detect deps once — used for both drift and dep count
    let deps_data = if !changed_only && check_dep_drift {
        detect::detect_deps(project_dir, true, &[], &[], false).ok()
    } else if changed_only {
        // changed_only path already called detect_deps above via check_drift
        None
    } else {
        detect::detect_deps(project_dir, false, &[], &[], false).ok()
    };

    // Drift
    if !changed_only {
        if check_dep_drift {
            if let Some(ref data) = deps_data {
                drift_info = extract_drift(data);
            }
        } else {
            drift_info = serde_json::json!({});
        }
    }
    let drifted_count = drift_info
        .get("changed")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let label = compute_label(
        total_rules,
        freshness_days,
        deterministic_coverage,
        drifted_count,
    );
    let score = compute_score(
        total_rules,
        freshness_days,
        deterministic_coverage,
        high_confidence_ratio,
        drifted_count,
    );

    let recommendations = build_recommendations(RecommendationInputs {
        total_rules,
        freshness_days,
        last_extraction_date: last_extraction_date.as_deref(),
        deterministic_coverage,
        drifted_count,
        drift_info: &drift_info,
        ai_signal_count: ai_count,
        unapproved_count,
    });

    // Project dep count — reuse whichever detect result is available
    let project_dep_count = deps_data
        .as_ref()
        .or(changed_only_deps_data.as_ref())
        .map(count_runtime_deps)
        .unwrap_or(0);

    let metrics = compute_impact_metrics(ImpactInputs {
        total_rules,
        approved_rules: &approved_rules,
        all_rules: &all_rules,
        dep_names: &dep_names,
        rule_files: &rule_files,
        deterministic_coverage,
        drifted_count,
        project_dep_count,
    });

    // Pipeline state
    let mut pipeline_state = serde_json::json!({});
    let mut cache_stats = serde_json::json!({});
    let mut extraction_readiness: Vec<Value> = Vec::new();
    let mut doc_stale: Vec<Value> = Vec::new();

    let mut sm = StateManager::new(project_dir);
    if let Ok(()) = {
        sm.load_all();

        let all_inv_deps = sm.inventory.all_deps();
        let runtime_inv_deps: Vec<&Value> = all_inv_deps
            .iter()
            .filter(|d| !d.get("dev").and_then(|v| v.as_bool()).unwrap_or(false))
            .collect();
        let mut state_counts: HashMap<String, usize> = HashMap::new();
        for d in &all_inv_deps {
            let s = d
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("discovered");
            *state_counts.entry(s.to_string()).or_insert(0) += 1;
        }

        // Use detected_totals from the last detect-deps run when available,
        // so that doctor and status agree on dependency counts.
        let detected_totals = sm.inventory.get_detected_totals();
        let (total_deps_count, runtime_deps_count) = if let Some(ref totals) = detected_totals {
            let total = totals
                .get("detected_total")
                .and_then(|v| v.as_i64())
                .unwrap_or(all_inv_deps.len() as i64);
            let runtime = totals
                .get("detected_runtime")
                .and_then(|v| v.as_i64())
                .unwrap_or(runtime_inv_deps.len() as i64);
            (total as usize, runtime as usize)
        } else {
            (all_inv_deps.len(), runtime_inv_deps.len())
        };

        pipeline_state = serde_json::json!({
            "total_deps": total_deps_count,
            "runtime_deps": runtime_deps_count,
            "inventory_entries": all_inv_deps.len(),
            "discovered": state_counts.get("discovered").unwrap_or(&0),
            "queued": state_counts.get("queued").unwrap_or(&0),
            "resolving": state_counts.get("resolving").unwrap_or(&0),
            "resolved": state_counts.get("resolved").unwrap_or(&0),
            "extraction_ready": state_counts.get("extraction_ready").unwrap_or(&0),
            "extracted": state_counts.get("extracted").unwrap_or(&0),
            "approved": state_counts.get("approved").unwrap_or(&0),
            "stale": state_counts.get("stale").unwrap_or(&0),
            "failed": state_counts.get("failed").unwrap_or(&0),
        });

        let cs = sm.cache.stats(None);
        cache_stats = serde_json::json!({
            "hits": cs.hits,
            "stale": cs.stale,
            "total": cs.total,
        });

        for d in &all_inv_deps {
            let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let language = d.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let version = d.get("version").and_then(|v| v.as_str()).unwrap_or("*");
            let mut entry = serde_json::json!({
                "name": name,
                "language": language,
                "state": d.get("state"),
            });
            if let Some(cached) = sm.cache.get(language, name, version) {
                entry["confidence"] = cached
                    .get("confidence")
                    .or_else(|| cached.get("freshness").and_then(|f| f.get("confidence")))
                    .cloned()
                    .unwrap_or(Value::Null);
                entry["source_type"] = cached.get("source_type").cloned().unwrap_or(Value::Null);
            }
            extraction_readiness.push(entry);
        }

        for cached_entry in sm.cache.all_entries() {
            let lang = cached_entry
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = cached_entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ver = cached_entry
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            if !sm.cache.is_fresh(lang, name, ver, Some(2592000)) {
                doc_stale.push(serde_json::json!({
                    "name": name,
                    "language": lang,
                    "fetch_timestamp": cached_entry.get("fetch_timestamp"),
                }));
            }
        }

        Ok::<(), ()>(())
    } {}

    // Next command
    let next_command = if drifted_count > 0 {
        "wh init --changed-only"
    } else if pipeline_state
        .get("failed")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        > 0
    {
        "wh set-sources --retry-failed"
    } else if pipeline_state
        .get("extraction_ready")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        > 0
    {
        "wh init --ready-only"
    } else if total_rules == 0 {
        "wh init"
    } else if freshness_days.map(|d| d > 30.0).unwrap_or(false) {
        "wh init --refresh"
    } else {
        "wh context && wh tests"
    };

    // Adherence score — "is my code in good shape?" (Epic 3E whetstone-90m).
    // Best-effort: we swallow errors so wh status stays snappy and non-blocking
    // when tree-sitter/check internals regress.
    let adherence = crate::adherence::compute(project_dir, total_rules).ok().flatten();
    let adherence_json = match &adherence {
        Some(a) => crate::adherence::to_json(a),
        None => serde_json::Value::Null,
    };
    let adherence_score: Option<i64> = adherence.as_ref().map(|a| a.score);

    Ok(serde_json::json!({
        "status": "ok",
        "label": label,
        "score": score,
        "rule_system_score": score,
        "adherence_score": adherence_score,
        "adherence": adherence_json,
        "freshness_label": freshness_label,
        "last_extraction_date": last_extraction_date,
        "dimensions": {
            "freshness_days": freshness_days.map(|d| (d * 10.0).round() / 10.0),
            "rules_count": total_rules,
            "high_confidence_ratio": (high_confidence_ratio * 10.0).round() / 10.0,
            "deterministic_coverage": (deterministic_coverage * 10.0).round() / 10.0,
            "pending_updates": drifted_count,
        },
        "breakdown": {
            "confidence": {"high": high_confidence, "medium": medium_confidence},
            "severity": {"must": must_count, "should": should_count, "may": may_count},
            "categories": categories,
            "signals": {"deterministic": deterministic_count, "ai": ai_count, "total": total_signals},
        },
        "dependencies_covered": dep_names.iter().collect::<Vec<_>>(),
        "drift": {
            "dependency_changes": drift_info.get("changed").cloned().unwrap_or(serde_json::json!([])),
            "documentation_stale": doc_stale,
            "count": drifted_count,
            "checked": drift_info.get("checked").and_then(|v| v.as_i64()).unwrap_or(0),
        },
        "pipeline_state": pipeline_state,
        "cache_stats": cache_stats,
        "extraction_readiness": extraction_readiness,
        "metrics": metrics,
        "recommendations": recommendations,
        "warnings": load_warnings,
        "next_command": next_command,
    }))
}

fn load_rule_files(rules_dir: &Path) -> (Vec<Value>, Vec<String>) {
    rules::load_rules_as_json(rules_dir)
}

fn compute_freshness_days(rule_files: &[Value]) -> Option<f64> {
    let mut latest: Option<DateTime<Utc>> = None;

    for rf in rule_files {
        if let Some(rules) = rf.get("rules").and_then(|v| v.as_array()) {
            for rule in rules {
                if let Some(ts) = rule.get("approved_at").and_then(|v| v.as_str()) {
                    let cleaned = ts.replace('Z', "+00:00");
                    if let Ok(dt) = DateTime::parse_from_rfc3339(&cleaned) {
                        let dt = dt.with_timezone(&Utc);
                        if latest.is_none() || dt > latest.unwrap() {
                            latest = Some(dt);
                        }
                    } else if let Ok(d) =
                        NaiveDate::parse_from_str(&cleaned[..10.min(cleaned.len())], "%Y-%m-%d")
                    {
                        let dt = d.and_hms_opt(0, 0, 0).unwrap().and_utc();
                        if latest.is_none() || dt > latest.unwrap() {
                            latest = Some(dt);
                        }
                    }
                }
            }
        }
    }

    latest.map(|dt| (Utc::now() - dt).num_seconds() as f64 / 86400.0)
}

fn compute_freshness_label(days: Option<f64>) -> String {
    match days {
        None => "Unknown".to_string(),
        Some(d) if d < 7.0 => "Fresh".to_string(),
        Some(d) if d < 30.0 => "Current".to_string(),
        Some(d) if d < 60.0 => "Aging".to_string(),
        _ => "Stale".to_string(),
    }
}

fn compute_last_extraction_date(rule_files: &[Value]) -> Option<String> {
    let days = compute_freshness_days(rule_files);
    days.map(|_| {
        // Re-derive from rule files
        let mut latest: Option<DateTime<Utc>> = None;
        for rf in rule_files {
            if let Some(rules) = rf.get("rules").and_then(|v| v.as_array()) {
                for rule in rules {
                    if let Some(ts) = rule.get("approved_at").and_then(|v| v.as_str()) {
                        let cleaned = ts.replace('Z', "+00:00");
                        if let Ok(dt) = DateTime::parse_from_rfc3339(&cleaned) {
                            let dt = dt.with_timezone(&Utc);
                            if latest.is_none() || dt > latest.unwrap() {
                                latest = Some(dt);
                            }
                        }
                    }
                }
            }
        }
        latest.map(|dt| dt.to_rfc3339()).unwrap_or_default()
    })
}

fn compute_label(
    total_rules: usize,
    freshness_days: Option<f64>,
    deterministic_coverage: f64,
    drifted_count: usize,
) -> String {
    if total_rules == 0 {
        return "No Rules".to_string();
    }
    if (freshness_days.is_some() && freshness_days.unwrap() > 60.0) || drifted_count >= 3 {
        return "Stale".to_string();
    }
    if drifted_count > 0 {
        return "Needs Review".to_string();
    }
    if freshness_days.is_some() && freshness_days.unwrap() > 30.0 {
        return "Needs Review".to_string();
    }
    if deterministic_coverage < 50.0 {
        return "Needs Review".to_string();
    }
    "Healthy".to_string()
}

fn compute_score(
    total_rules: usize,
    freshness_days: Option<f64>,
    deterministic_coverage: f64,
    high_confidence_ratio: f64,
    drifted_count: usize,
) -> i64 {
    if total_rules == 0 {
        return 0;
    }

    let freshness_score = match freshness_days {
        None => 15,
        Some(d) if d <= 7.0 => 30,
        Some(d) if d <= 30.0 => 25,
        Some(d) if d <= 60.0 => 15,
        Some(d) if d <= 90.0 => 5,
        _ => 0,
    };

    let det_score = (deterministic_coverage * 0.3).min(30.0) as i64;
    let conf_score = (high_confidence_ratio * 0.2).min(20.0) as i64;
    let drift_score = match drifted_count {
        0 => 20,
        1..=2 => 10,
        3..=5 => 5,
        _ => 0,
    };

    (freshness_score + det_score + conf_score + drift_score).min(100)
}

fn build_recommendations(inputs: RecommendationInputs<'_>) -> Vec<Value> {
    let RecommendationInputs {
        total_rules,
        freshness_days,
        last_extraction_date,
        deterministic_coverage,
        drifted_count,
        drift_info,
        ai_signal_count,
        unapproved_count,
    } = inputs;
    let mut recs = Vec::new();

    if total_rules == 0 {
        recs.push(serde_json::json!({"priority": "high", "action": "init", "message": "No rules found. Run 'wh init' to get started.", "command": "wh init"}));
        return recs;
    }

    if drifted_count > 0 {
        let changed = drift_info
            .get("changed")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let dep_list: Vec<String> = changed
            .iter()
            .take(3)
            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();
        let suffix = if drifted_count > 3 {
            format!(" (+{} more)", drifted_count - 3)
        } else {
            String::new()
        };
        recs.push(serde_json::json!({
            "priority": "high", "action": "refresh",
            "message": format!("{} deps have version drift: {}{suffix}. Re-run doctor to resolve updated sources.", drifted_count, dep_list.join(", ")),
            "command": "wh init --changed-only",
        }));
    }

    if let Some(days) = freshness_days {
        if days > 30.0 {
            let date_str = last_extraction_date
                .map(|d| format!(" (last: {})", &d[..10.min(d.len())]))
                .unwrap_or_default();
            recs.push(serde_json::json!({
                "priority": if days > 60.0 { "high" } else { "medium" },
                "action": "refresh",
                "message": format!("Last extraction was {:.0} days ago{date_str}. Re-run doctor to check for documentation changes.", days),
                "command": "wh init --refresh",
            }));
        }
    }

    if deterministic_coverage < 70.0 && total_rules > 0 {
        recs.push(serde_json::json!({
            "priority": "medium", "action": "review",
            "message": format!("Deterministic signal coverage is {:.0}%. Consider adding AST/pattern signals.", deterministic_coverage),
            "command": null,
        }));
    }

    if ai_signal_count > 0 && deterministic_coverage < 50.0 {
        recs.push(serde_json::json!({
            "priority": "medium", "action": "decompose",
            "message": format!("{} signals use AI \u{2014} consider decomposing into deterministic checks.", ai_signal_count),
            "command": null,
        }));
    }

    if unapproved_count > 0 {
        recs.push(serde_json::json!({
            "priority": "medium", "action": "approve",
            "message": format!("{} proposed rules await approval. Run extraction to review.", unapproved_count),
            "command": null,
        }));
    }

    if recs.is_empty() {
        if let Some(days) = freshness_days {
            let days_until = (30.0 - days).max(1.0) as i64;
            recs.push(serde_json::json!({"priority": "low", "action": "none", "message": format!("Everything looks good. Next check recommended in {} days.", days_until), "command": null}));
        } else {
            recs.push(serde_json::json!({"priority": "low", "action": "none", "message": "Everything looks good. No action needed.", "command": null}));
        }
    }

    recs
}

fn compute_impact_metrics(inputs: ImpactInputs<'_>) -> Value {
    let ImpactInputs {
        total_rules,
        approved_rules,
        all_rules,
        dep_names,
        rule_files,
        deterministic_coverage,
        drifted_count,
        project_dep_count,
    } = inputs;
    let total_proposed = all_rules.len();
    let approval_rate = if total_proposed > 0 {
        total_rules as f64 / total_proposed as f64 * 100.0
    } else {
        0.0
    };
    let must_rules = approved_rules
        .iter()
        .filter(|r| r.get("severity").and_then(|v| v.as_str()) == Some("must"))
        .count();

    let mut deps_with_rules: BTreeSet<String> = BTreeSet::new();
    for rf in rule_files {
        if let Some(rules) = rf.get("rules").and_then(|v| v.as_array()) {
            if rules
                .iter()
                .any(|r| r.get("approved").and_then(|v| v.as_bool()).unwrap_or(false))
            {
                if let Some(name) = rf.get("source_name").and_then(|v| v.as_str()) {
                    deps_with_rules.insert(name.to_string());
                }
            }
        }
    }

    let deps_covered = deps_with_rules.len();
    let deps_total = if project_dep_count > 0 {
        project_dep_count
    } else {
        dep_names.len()
    };
    let dep_coverage = if deps_total > 0 {
        deps_covered as f64 / deps_total as f64 * 100.0
    } else {
        0.0
    };

    serde_json::json!({
        "rules_approved": total_rules,
        "rules_proposed": total_proposed,
        "approval_rate": (approval_rate * 10.0).round() / 10.0,
        "must_rules": must_rules,
        "dependencies_covered": deps_covered,
        "dependencies_total": deps_total,
        "dependency_coverage": (dep_coverage * 10.0).round() / 10.0,
        "deterministic_coverage": (deterministic_coverage * 10.0).round() / 10.0,
        "pending_drift": drifted_count,
    })
}

fn extract_drift(deps_data: &Value) -> Value {
    deps_data
        .get("drift")
        .cloned()
        .unwrap_or(serde_json::json!({"changed": [], "count": 0, "checked": 0}))
}

fn count_runtime_deps(deps_data: &Value) -> usize {
    deps_data
        .get("dependencies")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|d| !d.get("dev").and_then(|v| v.as_bool()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

pub fn format_human_output(result: &Value) -> String {
    let mut r = crate::output::ReportBuilder::new();

    if result.get("status").and_then(|v| v.as_str()) == Some("not_initialized") {
        r.top_border();
        r.line("Whetstone Status");
        r.section_header("");
        r.empty_line();
        r.line(result.get("message").and_then(|v| v.as_str()).unwrap_or(""));
        r.line(&format!(
            "Next: {}",
            result
                .get("next_command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        ));
        r.empty_line();
        r.bottom_border();
        return r.build();
    }

    let label = result
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let score = result.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
    let dims = result.get("dimensions").cloned().unwrap_or(Value::Null);
    let freshness_label = result
        .get("freshness_label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    let indicator = match label {
        "Healthy" => "OK",
        "Needs Review" => "!!",
        "Stale" => "XX",
        "No Rules" => "--",
        _ => "??",
    };

    r.top_border();
    r.line("Whetstone Status");
    r.section_header("");
    r.empty_line();

    let rules_count = dims
        .get("rules_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let drifted = dims
        .get("pending_updates")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let adherence_score = result.get("adherence_score").and_then(|v| v.as_i64());
    let adherence_part = match adherence_score {
        Some(a) => format!(" \u{00b7} Adherence: {}/100", a),
        None => String::from(" \u{00b7} Adherence: n/a"),
    };

    let mut status_line = format!(
        "[{}] {} \u{00b7} Rule system: {}/100{} \u{00b7} {} rules",
        indicator, label, score, adherence_part, rules_count
    );
    if drifted > 0 {
        status_line.push_str(&format!(" \u{00b7} {} deps drifted", drifted));
    }
    r.line(&status_line);

    // Adherence detail (when we have it)
    if let Some(ad) = result.get("adherence") {
        if !ad.is_null() {
            let clean = ad.get("clean_ratio").and_then(|v| v.as_i64()).unwrap_or(0);
            let sev = ad
                .get("severity_component")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let v = ad.get("violations");
            let must = v.and_then(|o| o.get("must")).and_then(|x| x.as_i64()).unwrap_or(0);
            let should = v.and_then(|o| o.get("should")).and_then(|x| x.as_i64()).unwrap_or(0);
            let may = v.and_then(|o| o.get("may")).and_then(|x| x.as_i64()).unwrap_or(0);
            r.line(&format!(
                "                (clean {}% \u{00b7} severity-weighted {} \u{00b7} violations: {} must, {} should, {} may)",
                clean, sev, must, should, may
            ));
        }
    }
    r.empty_line();

    // Dimensions
    r.section_header("Health Dimensions");
    r.empty_line();

    if let Some(freshness) = dims.get("freshness_days").and_then(|v| v.as_f64()) {
        r.line(&format!(
            "Freshness:              {:.0} days ({})",
            freshness, freshness_label
        ));
    } else {
        r.line(&format!(
            "Freshness:              No timestamps found ({})",
            freshness_label
        ));
    }
    r.line(&format!("Rules:                  {} approved", rules_count));
    r.line(&format!(
        "High confidence:        {:.0}%",
        dims.get("high_confidence_ratio")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    ));
    r.line(&format!(
        "Deterministic coverage: {:.0}%",
        dims.get("deterministic_coverage")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    ));
    r.line(&format!(
        "Pending updates:        {} deps with drift",
        drifted
    ));

    // Recommendations
    if let Some(recs) = result.get("recommendations").and_then(|v| v.as_array()) {
        if !recs.is_empty() {
            r.empty_line();
            r.section_header("Recommendations");
            r.empty_line();
            for rec in recs {
                let priority = rec
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("medium");
                let msg = rec.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let marker = match priority {
                    "high" => "[HIGH]",
                    "medium" => "[MED]",
                    _ => "[LOW]",
                };
                r.line(&format!("{} {}", marker, msg));
                if let Some(cmd) = rec.get("command").and_then(|v| v.as_str()) {
                    r.line(&format!("       -> {}", cmd));
                }
            }
            r.empty_line();
        }
    }

    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        r.line(&format!("Next: {}", next));
        r.empty_line();
    }

    r.bottom_border();
    r.build()
}

pub fn extraction_ready_list(project_dir: &Path) -> Vec<Value> {
    let mut sm = StateManager::new(project_dir);
    sm.inventory.load();
    sm.inventory
        .by_state("extraction_ready")
        .iter()
        .map(|d| serde_json::json!({"name": d.get("name"), "language": d.get("language")}))
        .collect()
}

pub fn snapshot_metrics(project_dir: &Path, result: &Value) {
    let metrics = match result.get("metrics") {
        Some(m) => m,
        None => return,
    };

    // Violation-trend snapshot (Epic 3E whetstone-m2q): carry the adherence
    // score AND violation counts per-severity forward so `wh status --history`
    // can compute deltas over time.
    let adherence = result.get("adherence").cloned().unwrap_or(Value::Null);
    let adherence_score = result.get("adherence_score").cloned().unwrap_or(Value::Null);
    let violation_counts = adherence
        .get("violations")
        .cloned()
        .unwrap_or(serde_json::json!({"must": 0, "should": 0, "may": 0, "total": 0}));

    let snapshot = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "score": result.get("score"),
        "rule_system_score": result.get("rule_system_score"),
        "adherence_score": adherence_score,
        "label": result.get("label"),
        "rules_approved": metrics.get("rules_approved"),
        "rules_proposed": metrics.get("rules_proposed"),
        "approval_rate": metrics.get("approval_rate"),
        "must_rules": metrics.get("must_rules"),
        "dependencies_covered": metrics.get("dependencies_covered"),
        "dependencies_total": metrics.get("dependencies_total"),
        "dependency_coverage": metrics.get("dependency_coverage"),
        "deterministic_coverage": metrics.get("deterministic_coverage"),
        "pending_drift": metrics.get("pending_drift"),
        "violation_counts": violation_counts,
    });

    let metrics_file = project_dir.join("whetstone").join(".metrics.jsonl");
    if let Some(parent) = metrics_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&metrics_file)
    {
        use std::io::Write;
        let _ = writeln!(
            f,
            "{}",
            serde_json::to_string(&snapshot).unwrap_or_default()
        );
    }
}

pub fn load_metrics_history(project_dir: &Path, limit: usize) -> Vec<Value> {
    let metrics_file = project_dir.join("whetstone").join(".metrics.jsonl");
    if !metrics_file.exists() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&metrics_file) {
        for line in text.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                entries.push(v);
            }
        }
    }

    let start = entries.len().saturating_sub(limit);
    entries[start..].to_vec()
}

pub fn format_history(entries: &[Value]) -> String {
    if entries.is_empty() {
        return "No metric history found. Run 'wh status' to record snapshots.".to_string();
    }

    let mut lines = Vec::new();
    lines.push(String::new());
    lines.push("=".repeat(72));
    lines.push("  Whetstone Metric History".to_string());
    lines.push("=".repeat(72));
    lines.push(format!(
        "  {:<12} {:>5}  {:<14} {:>5} {:>4} {:>5} {:>5}",
        "Date", "Score", "Label", "Rules", "Must", "Det%", "Drift"
    ));
    lines.push(format!("  {}", "-".repeat(64)));

    for entry in entries {
        let ts = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")[..10.min(
            entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .len(),
        )]
            .to_string();
        let score = entry.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
        let label = entry.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let rules = entry
            .get("rules_approved")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let must = entry
            .get("must_rules")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let det = entry
            .get("deterministic_coverage")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let drift = entry
            .get("pending_drift")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        lines.push(format!(
            "  {:<12} {:>5}  {:<14} {:>5} {:>4} {:>4.0}% {:>5}",
            ts, score, label, rules, must, det, drift
        ));
    }

    if entries.len() >= 2 {
        let first = &entries[0];
        let last = entries.last().unwrap();
        let score_delta = last.get("score").and_then(|v| v.as_i64()).unwrap_or(0)
            - first.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
        let rules_delta = last
            .get("rules_approved")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            - first
                .get("rules_approved")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
        lines.push(format!("  {}", "-".repeat(64)));
        lines.push(format!(
            "  Trend: score {}{}, rules {}{} over {} snapshots",
            if score_delta >= 0 { "+" } else { "" },
            score_delta,
            if rules_delta >= 0 { "+" } else { "" },
            rules_delta,
            entries.len()
        ));
    }

    lines.push("=".repeat(72));
    lines.push(String::new());
    lines.join("\n")
}
