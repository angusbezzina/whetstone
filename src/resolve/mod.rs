pub mod crates_io;
pub mod http;
pub mod npm;
pub mod pypi;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::state::StateManager;
use crate::types::LifecycleState;

pub struct ResolveOptions<'a> {
    pub deps_data: &'a Value,
    pub filter_deps: Option<&'a [String]>,
    pub changed_only: bool,
    pub project_dir: &'a Path,
    pub timeout: u64,
    pub ttl: u64,
    pub force_refresh: bool,
    pub resume: bool,
    pub retry_failed: bool,
    pub workers: Option<usize>,
}

/// Resolve documentation sources for all dependencies.
pub fn resolve_sources(options: ResolveOptions<'_>) -> Result<Value> {
    let ResolveOptions {
        deps_data,
        filter_deps,
        changed_only,
        project_dir,
        timeout,
        ttl,
        force_refresh,
        resume,
        retry_failed,
        workers,
    } = options;
    let start_time = Instant::now();
    let mut sources: Vec<Value> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    let mut cache_counts = serde_json::json!({"hit": 0, "miss": 0, "stale": 0});
    let mut skipped_cached = 0usize;
    let timings: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    let stored_hashes = load_stored_hashes(project_dir);

    let mut sm = StateManager::new(project_dir);
    sm.ensure_dir();
    sm.cache.load();
    sm.inventory.load();
    sm.refresh_log.load();

    // Build work list
    let mut work_list: Vec<Value> = Vec::new();
    let empty_deps = vec![];
    let deps = deps_data
        .get("dependencies")
        .and_then(|d| d.as_array())
        .unwrap_or(&empty_deps);

    for dep in deps {
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("*");
        let is_dev = dep.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);

        if let Some(filter) = filter_deps {
            if !filter.iter().any(|f| f == name) {
                continue;
            }
        }
        if is_dev {
            continue;
        }

        // Resume: skip already resolved
        if resume {
            if let Some(inv) = sm.inventory.get(language, name) {
                let state = inv.get("state").and_then(|s| s.as_str()).unwrap_or("");
                if matches!(
                    state,
                    "resolved" | "extraction_ready" | "extracted" | "approved"
                ) {
                    if let Some(cached) = sm.cache.get(language, name, version) {
                        if cache_entry_reusable(&cached) {
                            sources.push(cached);
                            skipped_cached += 1;
                            *cache_counts.get_mut("hit").unwrap() =
                                Value::from(cache_counts["hit"].as_i64().unwrap_or(0) + 1);
                            continue;
                        }
                    }
                }
            }
        }

        // Retry-failed: only process failed deps
        if retry_failed {
            if let Some(inv) = sm.inventory.get(language, name) {
                if inv.get("state").and_then(|s| s.as_str()) != Some("failed") {
                    continue;
                }
            }
        }

        // Check cache
        if !force_refresh {
            if let Some(cached) = sm.cache.get(language, name, version) {
                if cache_entry_reusable(&cached) {
                    if sm.cache.is_fresh(language, name, version, Some(ttl)) {
                        *cache_counts.get_mut("hit").unwrap() =
                            Value::from(cache_counts["hit"].as_i64().unwrap_or(0) + 1);
                        sources.push(cached);
                        skipped_cached += 1;
                        continue;
                    } else {
                        *cache_counts.get_mut("stale").unwrap() =
                            Value::from(cache_counts["stale"].as_i64().unwrap_or(0) + 1);
                    }
                } else {
                    *cache_counts.get_mut("stale").unwrap() =
                        Value::from(cache_counts["stale"].as_i64().unwrap_or(0) + 1);
                }
            } else {
                *cache_counts.get_mut("miss").unwrap() =
                    Value::from(cache_counts["miss"].as_i64().unwrap_or(0) + 1);
            }
        } else {
            *cache_counts.get_mut("miss").unwrap() =
                Value::from(cache_counts["miss"].as_i64().unwrap_or(0) + 1);
        }

        work_list.push(dep.clone());
    }

    // Mark deps as resolving
    for dep in &work_list {
        let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !sm
            .inventory
            .set_state(language, name, LifecycleState::Resolving)
        {
            sm.inventory.upsert_dep(dep);
            sm.inventory
                .set_state(language, name, LifecycleState::Resolving);
        }
    }
    sm.inventory.save();

    let total = work_list.len();
    let effective_workers = workers.unwrap_or_else(|| recommended_workers(total));
    let effective_workers = effective_workers.min(total).max(1);

    let pb = if crate::output::is_piped() || work_list.is_empty() {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("\u{2501}\u{257a}\u{2500}"),
        );
        pb.set_message("Resolving...");
        pb
    };
    let pb = std::sync::Arc::new(pb);

    // Thread-safe state for checkpointing
    let sm_mutex = Arc::new(Mutex::new(sm));
    let completed = Arc::new(Mutex::new(0usize));
    let results: Arc<Mutex<Vec<(Value, f64)>>> = Arc::new(Mutex::new(Vec::new()));

    // Parallel resolution
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(effective_workers)
        .build()
        .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

    pool.scope(|s| {
        for dep in &work_list {
            let dep = dep.clone();
            let stored_hashes = &stored_hashes;
            let sm_mutex = Arc::clone(&sm_mutex);
            let completed = Arc::clone(&completed);
            let results = Arc::clone(&results);
            let timings = Arc::clone(&timings);
            let pb = Arc::clone(&pb);

            s.spawn(move |_| {
                let started = Instant::now();
                let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
                let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("*");
                let lang_key = format!("{language}:{name}");
                let stored_hash = stored_hashes.get(&lang_key).map(|s| s.as_str());

                let result = resolve_single_dep(name, language, version, stored_hash, timeout);
                let elapsed = started.elapsed().as_secs_f64();

                // Checkpoint to state
                {
                    let mut sm = sm_mutex.lock().unwrap();
                    if result.get("error").is_some() {
                        sm.inventory
                            .set_state(language, name, LifecycleState::Failed);
                    } else {
                        // Check content hash change
                        if let Some(old_cached) = sm.cache.get(language, name, version) {
                            let old_hash = old_cached.get("content_hash").and_then(|v| v.as_str());
                            let new_hash = result.get("content_hash").and_then(|v| v.as_str());
                            if let (Some(oh), Some(nh)) = (old_hash, new_hash) {
                                if oh != nh {
                                    sm.refresh_log.record(
                                        "content_hash_changed",
                                        &format!("{language}:{name}"),
                                        "content changed on re-resolve",
                                    );
                                }
                            }
                        }

                        // Cache the result
                        let mut cache_entry = result.clone();
                        cache_entry["fetch_timestamp"] =
                            Value::String(chrono::Utc::now().to_rfc3339());
                        cache_entry["ttl_seconds"] = Value::from(ttl);
                        sm.cache.upsert(cache_entry);

                        // Determine extraction readiness
                        let source_type = result
                            .get("source_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let confidence = result
                            .get("freshness")
                            .and_then(|f| f.get("confidence"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("low");
                        let has_content = result
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|s| !s.is_empty())
                            .unwrap_or(false);

                        if matches!(source_type, "llms_full_txt" | "llms_txt")
                            && has_content
                            && confidence == "high"
                        {
                            sm.inventory
                                .set_state(language, name, LifecycleState::ExtractionReady);
                        } else {
                            sm.inventory
                                .set_state(language, name, LifecycleState::Resolved);
                        }
                    }
                    sm.cache.save();
                    sm.inventory.save();
                }

                let mut count = completed.lock().unwrap();
                *count += 1;
                let status_str = if result.get("error").is_some() {
                    "error".to_string()
                } else {
                    result
                        .get("source_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ok")
                        .to_string()
                };
                pb.inc(1);
                pb.set_message(format!("{name}: {status_str}"));

                timings.lock().unwrap().push(serde_json::json!({
                    "name": name,
                    "language": language,
                    "elapsed_seconds": (elapsed * 1000.0).round() / 1000.0,
                    "status": if result.get("error").is_some() { "error" } else { "ok" },
                    "source_type": result.get("source_type"),
                }));

                results.lock().unwrap().push((result, elapsed));
            });
        }
    });

    pb.finish_and_clear();

    // Collect results
    let all_results = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    for (result, _elapsed) in all_results {
        if changed_only {
            if let Some(hash) = result.get("content_hash").and_then(|v| v.as_str()) {
                let name = result.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let language = result
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let lang_key = format!("{language}:{name}");
                let stored = stored_hashes.get(&lang_key);
                if let Some(stored) = stored {
                    if stored == hash {
                        continue;
                    }
                }
            }
        }

        if result.get("error").is_some() {
            errors.push(serde_json::json!({
                "name": result.get("name"),
                "language": result.get("language"),
                "error": result.get("error"),
            }));
        } else {
            sources.push(result);
        }
    }

    // Save final state
    {
        let sm = sm_mutex.lock().unwrap();
        sm.refresh_log.save();
    }

    let next_command = if !sources.is_empty() {
        "Extract rules: agent applies extraction prompt to each source"
    } else {
        "No sources resolved. Provide manual docs URLs or check errors above."
    };

    let wall_seconds = start_time.elapsed().as_secs_f64();
    let timings_vec = Arc::try_unwrap(timings).unwrap().into_inner().unwrap();

    // Build source type timing stats
    let mut by_source_type: HashMap<String, (usize, f64)> = HashMap::new();
    for t in &timings_vec {
        let st = t
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        let elapsed = t
            .get("elapsed_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let entry = by_source_type.entry(st.to_string()).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += elapsed;
    }

    let by_source_type_json: Value = by_source_type
        .iter()
        .map(|(k, (count, total))| {
            (k.clone(), serde_json::json!({
                "count": count,
                "total_seconds": (total * 1000.0).round() / 1000.0,
                "average_seconds": if *count > 0 { ((total / *count as f64) * 1000.0).round() / 1000.0 } else { 0.0 },
            }))
        })
        .collect::<serde_json::Map<String, Value>>()
        .into();

    let mut slowest: Vec<&Value> = timings_vec.iter().collect();
    slowest.sort_by(|a, b| {
        let a_e = a
            .get("elapsed_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let b_e = b
            .get("elapsed_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        b_e.partial_cmp(&a_e).unwrap_or(std::cmp::Ordering::Equal)
    });
    let slowest: Vec<Value> = slowest.into_iter().take(10).cloned().collect();

    Ok(serde_json::json!({
        "sources": sources,
        "errors": errors,
        "cache": cache_counts,
        "resolution_stats": {
            "total": total + skipped_cached,
            "resolved": sources.len(),
            "failed": errors.len(),
            "skipped_cached": skipped_cached,
            "workers": effective_workers,
            "wall_seconds": (wall_seconds * 1000.0).round() / 1000.0,
            "timings": {
                "by_source_type": by_source_type_json,
                "slowest_dependencies": slowest,
            },
        },
        "next_command": next_command,
    }))
}

fn resolve_single_dep(
    name: &str,
    language: &str,
    version: &str,
    stored_hash: Option<&str>,
    timeout: u64,
) -> Value {
    let result = match language {
        "python" => pypi::resolve(name, version, timeout),
        "typescript" => npm::resolve(name, version, timeout),
        "rust" => crates_io::resolve(name, version, timeout),
        _ => serde_json::json!({"error": format!("Unsupported language: {language}")}),
    };

    if result.get("error").is_some() {
        return serde_json::json!({
            "name": name,
            "language": language,
            "version": version,
            "error": result["error"],
        });
    }

    let freshness = compute_freshness(&result, stored_hash);

    let mut out = serde_json::json!({
        "name": name,
        "language": language,
        "version": version,
    });
    // Merge result fields
    if let (Some(out_obj), Some(res_obj)) = (out.as_object_mut(), result.as_object()) {
        for (k, v) in res_obj {
            out_obj.insert(k.clone(), v.clone());
        }
    }
    out["freshness"] = freshness;
    out
}

fn compute_freshness(result: &Value, stored_hash: Option<&str>) -> Value {
    let mut source_age_days: Value = Value::Null;
    let mut content_stale = false;
    // Source age from latest_release_date
    if let Some(date_str) = result.get("latest_release_date").and_then(|v| v.as_str()) {
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&date_str.replace('Z', "+00:00")) {
            let age = chrono::Utc::now() - parsed.with_timezone(&chrono::Utc);
            source_age_days = Value::from(age.num_days());
        } else if let Ok(parsed) =
            chrono::NaiveDate::parse_from_str(&date_str[..10.min(date_str.len())], "%Y-%m-%d")
        {
            let today = chrono::Utc::now().date_naive();
            let age = today - parsed;
            source_age_days = Value::from(age.num_days());
        }
    }

    // Confidence based on source type
    let source_type = result
        .get("source_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let confidence = match source_type {
        "llms_full_txt" | "llms_txt" => "high",
        "readme" | "html_converted" => "medium",
        "docs_url_only" => "low",
        _ if result
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false) =>
        {
            "medium"
        }
        _ => "low",
    };

    // Content staleness
    if let (Some(stored), Some(current)) = (
        stored_hash,
        result.get("content_hash").and_then(|v| v.as_str()),
    ) {
        content_stale = stored != current;
    }

    serde_json::json!({
        "source_age_days": source_age_days,
        "content_stale": content_stale,
        "confidence": confidence,
    })
}

fn cache_entry_reusable(entry: &Value) -> bool {
    if entry
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
    {
        return false;
    }
    let source_type = entry
        .get("source_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(source_type, "llms_full_txt" | "llms_txt") {
        return entry
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
    }
    true
}

fn recommended_workers(total: usize) -> usize {
    if total <= 4 {
        total.max(1)
    } else if total <= 12 {
        6
    } else {
        12.min(total)
    }
}

/// Format resolve result as a human-readable summary.
pub fn format_human_output(result: &Value) -> String {
    let mut lines = Vec::new();

    let total = result
        .get("resolved")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let llms_count = result
        .get("resolved")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|d| {
                    matches!(
                        d.get("source_type").and_then(|v| v.as_str()),
                        Some("llms_full_txt" | "llms_txt")
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let failed = result
        .get("failed")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    lines.push(format!("Resolved {total} dependencies ({llms_count} with llms.txt)"));
    if failed > 0 {
        lines.push(format!("{failed} failed to resolve"));
    }

    if let Some(resolved) = result.get("resolved").and_then(|v| v.as_array()) {
        for dep in resolved {
            let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let src = dep
                .get("source_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let icon = match src {
                "llms_full_txt" => "\u{2713}\u{2713}",
                "llms_txt" => "\u{2713}",
                "docs_url_only" => "\u{25cb}",
                _ => "?",
            };
            lines.push(format!("  {icon} {name} ({src})"));
        }
    }

    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        lines.push(String::new());
        lines.push(format!("Next: {next}"));
    }

    lines.join("\n")
}

pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn load_stored_hashes(project_dir: &Path) -> HashMap<String, String> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (loaded, _warnings) = crate::rules::load_rule_files(&rules_dir);
    let mut hashes = HashMap::new();

    for lrf in &loaded {
        let name = &lrf.rule_file.source.name;
        let lang = lrf.language.as_deref().unwrap_or("unknown");
        if let Some(ref hash) = lrf.rule_file.source.content_hash {
            if !name.is_empty() && !hash.is_empty() {
                // Key by language:name to avoid cross-ecosystem collisions
                hashes.insert(format!("{lang}:{name}"), hash.clone());
                // Also store by name-only for backwards compatibility
                hashes.insert(name.clone(), hash.clone());
            }
        }
    }

    hashes
}

/// Probe for llms-full.txt and llms.txt at a base URL.
pub fn probe_llms_txt(base_url: &str, timeout: u64) -> (Option<String>, Option<String>, String) {
    let base = base_url.trim_end_matches('/');
    let suffixes = ["", "/latest", "/stable", "/en/latest", "/en/stable"];

    for suffix in &suffixes {
        let root = format!("{base}{suffix}");
        for (path, source_type) in [
            (format!("{root}/llms-full.txt"), "llms_full_txt"),
            (format!("{root}/llms.txt"), "llms_txt"),
        ] {
            if let Some(content) = http::http_get_plain_text(&path, timeout) {
                if content.len() > 50 {
                    return (Some(content), Some(path), source_type.to_string());
                }
            }
        }
    }

    (None, None, "none".to_string())
}
