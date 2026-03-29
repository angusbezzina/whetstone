use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use crate::detect;
use crate::output::{self, ReportBuilder};
use crate::resolve;
use crate::state::StateManager;

const DEFAULT_FAST_FIRST_MAX_DEPS: usize = 10;

pub struct DoctorOptions<'a> {
    pub project_dir: &'a Path,
    pub skip_dev: bool,
    pub json_mode: bool,
    pub deps_filter: Option<&'a str>,
    pub verbose: bool,
    pub changed_only: bool,
    pub refresh: bool,
    pub resume: bool,
    pub max_deps: Option<usize>,
    pub ready_only: bool,
    pub workers: Option<usize>,
    pub full_run: bool,
}

pub fn doctor(options: DoctorOptions<'_>) -> Result<Value> {
    let DoctorOptions {
        project_dir,
        skip_dev,
        json_mode,
        deps_filter,
        verbose,
        changed_only,
        refresh,
        resume,
        max_deps,
        ready_only,
        workers,
        full_run,
    } = options;
    let total_start = Instant::now();
    let mut steps: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let mut sm = StateManager::new(project_dir);
    sm.ensure_dir();
    sm.load_all();

    let existing_rules = count_existing_rules(project_dir);

    // ── Step 1: Detect dependencies ──
    log("Step 1/3: Detecting dependencies...", json_mode);
    let detect_start = Instant::now();
    let deps_result = detect::detect_deps(project_dir, false, &[], &[], true)?;
    let deps_time = detect_start.elapsed().as_secs_f64();

    if deps_result.get("error").is_some() {
        let error_msg = deps_result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error detecting dependencies");
        return Ok(serde_json::json!({
            "status": "error",
            "error": error_msg,
            "step": "detect-deps",
            "steps": steps,
            "recommendations": [],
            "source_details": [],
            "next_command": "Check project directory has manifest files (pyproject.toml, package.json, Cargo.toml)",
        }));
    }

    let deps_count = deps_result
        .get("counts")
        .and_then(|c| c.get("runtime"))
        .and_then(|r| r.get("_all"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dev_count = deps_result
        .get("counts")
        .and_then(|c| c.get("dev"))
        .and_then(|r| r.get("_all"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let languages: Vec<String> = deps_result
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let lang_counts: BTreeMap<String, i64> = languages
        .iter()
        .map(|lang| {
            let count = deps_result
                .get("counts")
                .and_then(|c| c.get("runtime"))
                .and_then(|r| r.get(lang.as_str()))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (lang.clone(), count)
        })
        .collect();

    steps.push(serde_json::json!({
        "name": "detect-deps",
        "status": "ok",
        "elapsed_seconds": (deps_time * 10.0).round() / 10.0,
        "summary": format!("Found {} runtime deps (+{} dev) across {}", deps_count, dev_count, languages.join(", ")),
    }));

    log(
        &format!(
            "  Found {} runtime dependencies (+{} dev) across {}  [{:.1}s]",
            deps_count,
            dev_count,
            languages.join(", "),
            deps_time
        ),
        json_mode,
    );

    // Filter deps
    let all_deps = deps_result
        .get("dependencies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut target_deps: Vec<Value> = if skip_dev {
        all_deps
            .iter()
            .filter(|d| !d.get("dev").and_then(|v| v.as_bool()).unwrap_or(false))
            .cloned()
            .collect()
    } else {
        all_deps.clone()
    };

    if let Some(filter) = deps_filter {
        let filter_set: Vec<&str> = filter.split(',').collect();
        target_deps.retain(|d| {
            d.get("name")
                .and_then(|v| v.as_str())
                .map(|n| filter_set.contains(&n))
                .unwrap_or(false)
        });
    }

    if target_deps.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "warning": "No dependencies to extract rules for",
            "steps": steps,
            "summary": {
                "dependencies_found": deps_count,
                "dependencies_targeted": 0,
                "sources_resolved": 0,
                "patterns_found": 0,
                "languages": languages,
            },
            "recommendations": [],
            "source_details": [],
            "scan": {"cache_stats": {}, "ranked_queue": []},
            "next_command": "Add dependencies to your project, then run whetstone doctor again",
        }));
    }

    // Reload state (detect may have updated it)
    sm.cache.load();
    sm.inventory.load();

    // Classify and rank
    let cache_buckets = classify_deps(&target_deps, &mut sm);
    let ranked_queue = rank_dependencies(&target_deps, &mut sm);

    let scan_info = serde_json::json!({
        "manifests_changed": deps_result.get("manifests_changed"),
        "manifest_diff": deps_result.get("manifest_diff"),
        "inventory_diff": deps_result.get("inventory_diff"),
        "cache_stats": {
            "cached": cache_buckets.cached.len(),
            "stale": cache_buckets.stale.len(),
            "missing": cache_buckets.missing.len(),
            "failed": cache_buckets.failed.len(),
        },
        "ranked_queue": ranked_queue.iter().take(20).map(|d| {
            serde_json::json!({
                "name": d.get("name"),
                "language": d.get("language"),
                "score": d.get("_score"),
            })
        }).collect::<Vec<_>>(),
    });

    log(
        &format!(
            "  Scan: {} cached, {} stale, {} missing, {} failed",
            cache_buckets.cached.len(),
            cache_buckets.stale.len(),
            cache_buckets.missing.len(),
            cache_buckets.failed.len()
        ),
        json_mode,
    );

    // Build resolve work list
    let ranked_key_order: Vec<String> = ranked_queue
        .iter()
        .filter_map(|d| {
            let lang = d.get("language").and_then(|v| v.as_str())?;
            let name = d.get("name").and_then(|v| v.as_str())?;
            Some(format!("{lang}:{name}"))
        })
        .collect();

    let mut resolve_deps: Vec<Value> = if changed_only {
        let mut deps: Vec<Value> = cache_buckets.stale.clone();
        deps.extend(cache_buckets.missing.clone());

        // Include deps whose manifests changed
        let inv_diff = deps_result
            .get("inventory_diff")
            .cloned()
            .unwrap_or(Value::Null);
        let changed_keys: Vec<String> = inv_diff
            .get("changed")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .chain(
                inv_diff
                    .get("added")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten(),
            )
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if !changed_keys.is_empty() {
            for dep in &cache_buckets.cached {
                let lang = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
                let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let key = format!("{lang}:{name}");
                if changed_keys.contains(&key) {
                    deps.push(dep.clone());
                }
            }
        }
        deps
    } else {
        target_deps.clone()
    };

    // Sort by ranked order
    let resolve_map: BTreeMap<String, Value> = resolve_deps
        .iter()
        .filter_map(|d| {
            let lang = d.get("language").and_then(|v| v.as_str())?;
            let name = d.get("name").and_then(|v| v.as_str())?;
            Some((format!("{lang}:{name}"), d.clone()))
        })
        .collect();

    resolve_deps = ranked_key_order
        .iter()
        .filter_map(|k| resolve_map.get(k).cloned())
        .collect();

    // Fast-first limiting
    let mut auto_limited = false;
    if max_deps.is_none()
        && !full_run
        && !changed_only
        && !refresh
        && !resume
        && !ready_only
        && deps_filter.is_none()
        && cache_buckets.cached.is_empty()
        && resolve_deps.len() > DEFAULT_FAST_FIRST_MAX_DEPS
    {
        resolve_deps.truncate(DEFAULT_FAST_FIRST_MAX_DEPS);
        auto_limited = true;
        log(
            &format!(
                "  Fast-first: limiting initial resolution to top {} deps; use --full-run or --resume to continue",
                DEFAULT_FAST_FIRST_MAX_DEPS
            ),
            json_mode,
        );
    }

    if let Some(max) = max_deps {
        resolve_deps.truncate(max);
    }

    // ── Step 2: Resolve sources ──
    log(
        &format!(
            "Step 2/3: Resolving documentation for {} dependencies...",
            resolve_deps.len()
        ),
        json_mode,
    );

    let resolve_start = Instant::now();
    let resolve_input = serde_json::json!({"dependencies": resolve_deps});
    let dep_names: Vec<String> = resolve_deps
        .iter()
        .filter_map(|d| d.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    let filter: Option<Vec<String>> =
        if !dep_names.is_empty() && dep_names.len() < target_deps.len() {
            Some(dep_names)
        } else {
            None
        };

    let resolve_result = resolve::resolve_sources(resolve::ResolveOptions {
        deps_data: &resolve_input,
        filter_deps: filter.as_deref(),
        changed_only: false,
        project_dir,
        timeout: 15,
        ttl: 604800,
        force_refresh: refresh,
        resume,
        retry_failed: false,
        workers,
    })?;
    let resolve_time = resolve_start.elapsed().as_secs_f64();

    let sources: Vec<Value> = resolve_result
        .get("sources")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let errors: Vec<Value> = resolve_result
        .get("errors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let llms_txt_count = sources
        .iter()
        .filter(|s| {
            matches!(
                s.get("source_type").and_then(|v| v.as_str()),
                Some("llms_txt" | "llms_full_txt")
            )
        })
        .count();

    steps.push(serde_json::json!({
        "name": "resolve-sources",
        "status": "ok",
        "elapsed_seconds": (resolve_time * 10.0).round() / 10.0,
        "summary": format!("Resolved docs for {}/{} deps ({} with llms.txt)", sources.len(), target_deps.len(), llms_txt_count),
    }));

    for err in &errors {
        let name = err.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let error = err
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        warnings.push(format!("Could not resolve docs for {name}: {error}"));
    }

    log(
        &format!(
            "  Resolved {}/{} deps ({} with llms.txt)  [{:.1}s]",
            sources.len(),
            target_deps.len(),
            llms_txt_count,
            resolve_time
        ),
        json_mode,
    );

    // ── Step 3: Extraction handoff ──
    let patterns_count = 0;
    log("Step 3/3: Preparing extraction handoff...", json_mode);

    let ready_now: Vec<&Value> = sources
        .iter()
        .filter(|s| {
            matches!(
                s.get("source_type").and_then(|v| v.as_str()),
                Some("llms_full_txt" | "llms_txt")
            ) && s
                .get("freshness")
                .and_then(|f| f.get("confidence"))
                .and_then(|c| c.as_str())
                == Some("high")
        })
        .collect();
    let resolved_low: Vec<&Value> = sources.iter().filter(|s| !ready_now.contains(s)).collect();

    let resolution_buckets = serde_json::json!({
        "ready_now": ready_now.iter().map(|s| serde_json::json!({"name": s.get("name"), "source_type": s.get("source_type")})).collect::<Vec<_>>(),
        "resolved_low": resolved_low.iter().map(|s| serde_json::json!({"name": s.get("name"), "source_type": s.get("source_type")})).collect::<Vec<_>>(),
        "failed": errors.iter().map(|e| serde_json::json!({"name": e.get("name"), "error": e.get("error")})).collect::<Vec<_>>(),
        "cached": cache_buckets.cached.iter().map(|d| serde_json::json!({"name": d.get("name")})).collect::<Vec<_>>(),
    });

    let extraction_subsets = serde_json::json!({
        "ready_now": ready_now.iter().map(|s| serde_json::json!({"name": s.get("name"), "source_type": s.get("source_type")})).collect::<Vec<_>>(),
        "resolved_not_ready": resolved_low.iter().map(|s| serde_json::json!({"name": s.get("name"), "reason": s.get("source_type")})).collect::<Vec<_>>(),
        "pending": cache_buckets.missing.iter().map(|d| serde_json::json!({"name": d.get("name")})).collect::<Vec<_>>(),
        "failed": errors.iter().map(|e| serde_json::json!({"name": e.get("name")})).collect::<Vec<_>>(),
    });

    let extraction_sources: Vec<Value> = if ready_only {
        ready_now.iter().map(|s| (*s).clone()).collect()
    } else {
        sources.clone()
    };

    let extraction_context = serde_json::json!({
        "sources": extraction_sources,
        "patterns": [],
        "languages": languages,
        "dep_names": extraction_sources.iter().filter_map(|s| s.get("name").and_then(|v| v.as_str())).collect::<Vec<_>>(),
    });

    steps.push(serde_json::json!({
        "name": "extraction-ready",
        "status": "ok",
        "elapsed_seconds": 0,
        "summary": format!("Ready for extraction: {} sources, {} patterns", extraction_sources.len(), patterns_count),
    }));

    let total_elapsed = total_start.elapsed().as_secs_f64();
    let source_details = build_source_details(&sources, &errors);
    let remaining_count = target_deps.len().saturating_sub(resolve_deps.len());
    let recommendations = build_recommendations(
        &sources,
        &errors,
        llms_txt_count,
        existing_rules,
        auto_limited,
        remaining_count,
    );

    let next_command = if auto_limited {
        "whetstone doctor --resume"
    } else if !extraction_sources.is_empty() {
        "Review extraction results, then: whetstone generate-context"
    } else if !sources.is_empty() {
        "whetstone status"
    } else {
        "whetstone doctor --refresh"
    };

    let result = serde_json::json!({
        "status": "ok",
        "steps": steps,
        "summary": {
            "dependencies_found": deps_count,
            "dependencies_targeted": target_deps.len(),
            "sources_resolved": sources.len(),
            "sources_with_llms_txt": llms_txt_count,
            "patterns_found": patterns_count,
            "languages": languages,
            "elapsed_seconds": (total_elapsed * 10.0).round() / 10.0,
        },
        "source_details": source_details,
        "recommendations": recommendations,
        "extraction_context": extraction_context,
        "scan": scan_info,
        "resolution_buckets": resolution_buckets,
        "extraction_subsets": extraction_subsets,
        "warnings": warnings,
        "next_command": next_command,
        "workflow": {
            "fast_first": auto_limited,
            "remaining_dependencies": remaining_count,
            "resolved_this_run": resolve_deps.len(),
        },
        "_existing_rules": existing_rules,
        "_dev_count": dev_count,
        "_lang_counts": lang_counts,
    });

    // Print human-readable report
    if !json_mode {
        let report = format_report(&result, project_dir, verbose);
        eprintln!("{report}");
    }

    Ok(result)
}

struct CacheBuckets {
    cached: Vec<Value>,
    stale: Vec<Value>,
    missing: Vec<Value>,
    failed: Vec<Value>,
}

fn classify_deps(deps: &[Value], sm: &mut StateManager) -> CacheBuckets {
    let mut cached = Vec::new();
    let mut stale = Vec::new();
    let mut missing = Vec::new();
    let mut failed = Vec::new();

    for dep in deps {
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("*");

        match sm.cache.get(language, name, version) {
            None => missing.push(dep.clone()),
            Some(entry) => {
                if entry
                    .get("errors")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
                {
                    failed.push(dep.clone());
                } else if sm.cache.is_fresh(language, name, version, None) {
                    cached.push(dep.clone());
                } else {
                    stale.push(dep.clone());
                }
            }
        }
    }

    CacheBuckets {
        cached,
        stale,
        missing,
        failed,
    }
}

fn rank_dependencies(deps: &[Value], sm: &mut StateManager) -> Vec<Value> {
    let mut scored: Vec<Value> = deps
        .iter()
        .map(|dep| {
            let mut score = 0.0f64;
            let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("*");
            let is_dev = dep.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);

            if !is_dev {
                score += 100.0;
            }

            if let Some(cached) = sm.cache.get(language, name, version) {
                match cached.get("source_type").and_then(|v| v.as_str()) {
                    Some("llms_full_txt") => score += 50.0,
                    Some("llms_txt") => score += 40.0,
                    Some("docs_url_only") => score += 10.0,
                    _ => {}
                }
            }

            let sources_count = dep
                .get("sources")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            score += (sources_count.saturating_sub(1) * 20) as f64;

            if let Some(inv) = sm.inventory.get(language, name) {
                match inv.get("state").and_then(|s| s.as_str()) {
                    Some("stale") => score += 30.0,
                    Some("failed") => score += 5.0,
                    _ => {}
                }
            }

            let mut entry = dep.clone();
            entry["_score"] = serde_json::json!(score);
            entry
        })
        .collect();

    scored.sort_by(|a, b| {
        let a_s = a.get("_score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b_s = b.get("_score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        b_s.partial_cmp(&a_s).unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
}

fn count_existing_rules(project_dir: &Path) -> usize {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (rule_files, _) = crate::rules::load_rules_as_json(&rules_dir);
    rule_files
        .iter()
        .filter_map(|rf| rf.get("rules").and_then(|v| v.as_array()))
        .flat_map(|rules| rules.iter())
        .filter(|r| r.get("approved").and_then(|v| v.as_bool()).unwrap_or(false))
        .count()
}

fn build_source_details(sources: &[Value], errors: &[Value]) -> Vec<Value> {
    let mut details: Vec<Value> = Vec::new();

    for s in sources {
        let source_type = s
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let confidence = match source_type {
            "llms_txt" | "llms_full_txt" => "high",
            "docs_url" | "readme" => "medium",
            _ => "low",
        };
        details.push(serde_json::json!({
            "name": s.get("name"),
            "source_type": source_type,
            "confidence": confidence,
            "status": "resolved",
        }));
    }

    for e in errors {
        details.push(serde_json::json!({
            "name": e.get("name"),
            "source_type": null,
            "confidence": null,
            "status": "failed",
            "error": e.get("error"),
        }));
    }

    details.sort_by_key(|d| match d.get("confidence").and_then(|v| v.as_str()) {
        Some("high") => 0,
        Some("medium") => 1,
        Some("low") => 2,
        _ => 3,
    });

    details
}

fn build_recommendations(
    sources: &[Value],
    errors: &[Value],
    llms_txt_count: usize,
    existing_rules: usize,
    auto_limited: bool,
    remaining_count: usize,
) -> Vec<Value> {
    let mut recs = Vec::new();

    if !sources.is_empty() {
        recs.push(serde_json::json!({
            "priority": "high",
            "action": "extract",
            "message": format!("Extract rules for {} dependencies with resolved docs", sources.len()),
        }));
    }

    if llms_txt_count > 0 {
        recs.push(serde_json::json!({
            "priority": "high",
            "action": "prioritize",
            "message": format!("{} deps have llms.txt \u{2014} these will produce highest quality rules", llms_txt_count),
        }));
    }

    if !errors.is_empty() {
        recs.push(serde_json::json!({
            "priority": "medium",
            "action": "resolve",
            "message": format!("Consider providing manual docs URLs for {} unresolved dependencies", errors.len()),
        }));
    }

    if existing_rules > 0 {
        recs.push(serde_json::json!({
            "priority": "low",
            "action": "review",
            "message": format!("{} existing rules found \u{2014} doctor will update them", existing_rules),
        }));
    }

    if sources.is_empty() && errors.is_empty() {
        recs.push(serde_json::json!({
            "priority": "high",
            "action": "add-deps",
            "message": "No dependencies found. Add dependencies to your project first.",
        }));
    }

    if auto_limited && remaining_count > 0 {
        recs.push(serde_json::json!({
            "priority": "high",
            "action": "continue",
            "message": format!("Fast-first mode resolved the top {} dependencies; resume to continue with {} remaining", sources.len(), remaining_count),
            "command": "whetstone doctor --resume",
        }));
    }

    recs
}

fn format_report(result: &Value, project_dir: &Path, verbose: bool) -> String {
    let mut r = ReportBuilder::new();
    let summary = result.get("summary").cloned().unwrap_or(Value::Null);
    let existing_rules = result
        .get("_existing_rules")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    r.top_border();
    if existing_rules > 0 {
        r.line(&format!(
            "Whetstone Doctor Report (Update \u{2014} {} existing rules)",
            existing_rules
        ));
    } else {
        r.line("Whetstone Doctor Report");
    }
    r.section_header("");
    r.empty_line();
    r.line(&format!(
        "Project: {}",
        project_dir
            .canonicalize()
            .unwrap_or_else(|_| project_dir.to_path_buf())
            .display()
    ));
    r.line(&format!(
        "Date:    {}",
        chrono::Utc::now().format("%Y-%m-%d")
    ));
    r.empty_line();

    // Dependencies
    r.section_header("Dependencies");
    r.empty_line();
    let deps_found = summary
        .get("dependencies_found")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dev_count = result
        .get("_dev_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let languages: Vec<&str> = summary
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    r.line(&format!(
        "Found {} runtime + {} dev dependencies",
        deps_found, dev_count
    ));
    r.line(&format!(
        "Languages: {}",
        if languages.is_empty() {
            "none".to_string()
        } else {
            languages.join(", ")
        }
    ));
    r.empty_line();

    // Sources
    r.section_header("Documentation Sources");
    r.empty_line();
    let sources_resolved = summary
        .get("sources_resolved")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let deps_targeted = summary
        .get("dependencies_targeted")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let llms_count = summary
        .get("sources_with_llms_txt")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    r.line(&format!(
        "Resolved: {}/{} dependencies",
        sources_resolved, deps_targeted
    ));
    r.line(&format!("With llms.txt: {}", llms_count));
    r.line(&format!("Docs URL only: {}", sources_resolved - llms_count));
    r.empty_line();

    // Source details
    let source_details = result.get("source_details").and_then(|v| v.as_array());
    if let Some(details) = source_details {
        if !details.is_empty() {
            r.line("Top sources:");
            let show_count = if verbose {
                details.len()
            } else {
                5.min(details.len())
            };
            for detail in details.iter().take(show_count) {
                let name = detail
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                if detail.get("status").and_then(|v| v.as_str()) == Some("resolved") {
                    let stype = detail
                        .get("source_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let conf = detail
                        .get("confidence")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    r.line(&format!(
                        "  + {:<16} -- {} ({} confidence)",
                        name, stype, conf
                    ));
                } else {
                    r.line(&format!("  x {:<16} -- no docs found", name));
                }
            }
            if !verbose && details.len() > 5 {
                r.line(&format!(
                    "  ... and {} more (use --verbose to show all)",
                    details.len() - 5
                ));
            }
            r.empty_line();
        }
    }

    // Timing
    r.section_header("Timing");
    r.empty_line();
    if let Some(steps) = result.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            let name = step
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let time = step
                .get("elapsed_seconds")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let indicator = match status {
                "ok" => "+",
                "skipped" => "~",
                _ => "x",
            };
            r.line(&format!("  {} {:<22} {:>5.1}s", indicator, name, time));
        }
    }
    let elapsed = summary
        .get("elapsed_seconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    r.line(&format!("  {:<24} {:>5.1}s", "Total:", elapsed));
    r.empty_line();
    r.bottom_border();

    r.build()
}

fn log(msg: &str, json_mode: bool) {
    if !json_mode {
        output::log(msg);
    }
}
