pub mod python;
pub mod rust_lang;
pub mod typescript;
pub mod walk;

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::config::WhetstoneConfig;
use crate::state::manifest::ManifestStore;
use crate::state::StateManager;

/// Main detection logic. Returns structured JSON matching the Python contract.
pub fn detect_deps(
    project_dir: &Path,
    do_check_drift: bool,
    cli_excludes: &[String],
    cli_includes: &[String],
    incremental: bool,
) -> Result<Value> {
    let config = WhetstoneConfig::load(project_dir);
    let mut merged_excludes: Vec<String> = config.discovery.exclude.clone();
    merged_excludes.extend(cli_excludes.iter().cloned());
    let mut merged_includes: Vec<String> = config.discovery.include.clone();
    merged_includes.extend(cli_includes.iter().cloned());

    let manifest_files = walk::find_manifests(project_dir, &merged_excludes, &merged_includes);

    if manifest_files.is_empty() {
        let effective_excluded = effective_excluded_list(&merged_excludes);
        return Ok(serde_json::json!({
            "languages": [],
            "dependencies": [],
            "manifests": [],
            "discovery": {
                "excluded": effective_excluded,
                "included": merged_includes,
                "monorepo": false,
                "workspaces": [],
            },
            "error": "No manifest files found",
            "next_command": "Ensure project has pyproject.toml, package.json, or Cargo.toml",
        }));
    }

    let mut all_deps: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut manifests_found: Vec<String> = Vec::new();

    for (filepath, source) in &manifest_files {
        let rel_path = filepath
            .strip_prefix(project_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| filepath.to_string_lossy().to_string());
        manifests_found.push(rel_path.clone());

        let filename = filepath.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let parse_result = match filename {
            "pyproject.toml" => python::parse_pyproject_toml(filepath, source),
            "requirements.txt" => python::parse_requirements_txt(filepath, source),
            "package.json" => typescript::parse_package_json(filepath, source),
            "Cargo.toml" => rust_lang::parse_cargo_toml(filepath, source),
            _ => continue,
        };

        match parse_result {
            Ok(deps) => all_deps.extend(deps),
            Err(e) => warnings.push(format!("Error parsing {rel_path}: {e}")),
        }
    }

    // Deduplicate by (name, language, dev)
    let unique_deps = dedup_deps(&all_deps);
    let languages: BTreeSet<String> = unique_deps
        .iter()
        .filter_map(|d| d.get("language").and_then(|v| v.as_str()).map(String::from))
        .collect();

    // Counts
    let runtime: Vec<&Value> = unique_deps.iter().filter(|d| !is_dev(d)).collect();
    let dev: Vec<&Value> = unique_deps.iter().filter(|d| is_dev(d)).collect();
    let counts = build_counts(&unique_deps, &runtime, &dev, &languages);

    // Monorepo detection
    let manifest_dirs: BTreeSet<String> = manifests_found
        .iter()
        .filter_map(|m| {
            let p = Path::new(m);
            p.parent().map(|d| d.to_string_lossy().to_string())
        })
        .collect();
    let is_monorepo = manifest_dirs.len() > 1;
    if is_monorepo {
        eprintln!(
            "Monorepo detected: {} workspaces found",
            manifest_dirs.len()
        );
    }

    let effective_excluded = effective_excluded_list(&merged_excludes);

    let mut result = serde_json::json!({
        "languages": languages.iter().collect::<Vec<_>>(),
        "counts": counts,
        "dependencies": unique_deps,
        "manifests": manifests_found,
        "discovery": {
            "excluded": effective_excluded,
            "included": merged_includes,
            "monorepo": is_monorepo,
            "workspaces": if is_monorepo { manifest_dirs.iter().collect::<Vec<_>>() } else { vec![] },
        },
    });

    if !warnings.is_empty() {
        result["warnings"] = serde_json::json!(warnings);
    }

    // Incremental mode
    if incremental {
        let mut sm = StateManager::new(project_dir);
        sm.ensure_dir();
        sm.manifests.load();
        sm.inventory.load();
        sm.refresh_log.load();

        let mut current_fingerprints: HashMap<String, String> = HashMap::new();
        for (filepath, _source) in &manifest_files {
            let rel_path = filepath
                .strip_prefix(project_dir)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            current_fingerprints.insert(rel_path, ManifestStore::fingerprint_file(filepath));
        }

        let manifest_diff = sm.manifests.compare(&current_fingerprints);

        for (filepath, source) in &manifest_files {
            let rel_path = filepath
                .strip_prefix(project_dir)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(sha) = current_fingerprints.get(&rel_path) {
                sm.manifests.upsert(&rel_path, sha, source);
            }
        }
        sm.manifests.save();

        let inventory_diff = sm.inventory.bulk_upsert(&unique_deps);

        for changed_path in &manifest_diff.changed {
            sm.refresh_log
                .record("manifest_changed", changed_path, "sha256 changed");
        }
        for inv_key in &inventory_diff.changed {
            sm.refresh_log
                .record("version_changed", inv_key, "version changed");
        }

        // Clean up stale inventory entries (deps removed from manifests)
        let mut cleanup_removed = Vec::new();
        let mut cleanup_protected = Vec::new();
        if !inventory_diff.removed.is_empty() {
            let protected = approved_dep_keys(project_dir);
            let cleanup = sm.inventory.remove_stale(&inventory_diff, &protected);
            for key in &cleanup.removed {
                sm.refresh_log
                    .record("dependency_removed", key, "no longer in manifests");
            }
            cleanup_removed = cleanup.removed;
            cleanup_protected = cleanup.protected;
        }

        // Persist detected totals for status reconciliation
        sm.inventory.set_detected_totals(&serde_json::json!({
            "detected_runtime": runtime.len(),
            "detected_dev": dev.len(),
            "detected_total": unique_deps.len(),
        }));

        sm.inventory.save();
        sm.refresh_log.save();

        result["manifests_changed"] = serde_json::json!(
            !manifest_diff.changed.is_empty()
                || !manifest_diff.added.is_empty()
                || !manifest_diff.removed.is_empty()
        );
        result["manifest_diff"] = manifest_diff.to_json();
        result["inventory_diff"] = serde_json::json!({
            "added": inventory_diff.added,
            "changed": inventory_diff.changed,
            "removed": inventory_diff.removed,
            "unchanged": inventory_diff.unchanged,
            "actually_removed": cleanup_removed,
            "protected": cleanup_protected,
        });
    }

    // Add scope field to scoped npm packages
    if let Some(deps) = result.get_mut("dependencies").and_then(|v| v.as_array_mut()) {
        for dep in deps.iter_mut() {
            if let Some(name) = dep.get("name").and_then(|v| v.as_str()).map(String::from) {
                if name.starts_with('@') {
                    if let Some(slash_pos) = name.find('/') {
                        dep["scope"] = serde_json::json!(&name[..slash_pos]);
                    }
                }
            }
        }
    }

    if do_check_drift {
        let drift = check_drift(&unique_deps, project_dir);
        let drift_count = drift.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
        if drift_count > 0 {
            result["next_command"] =
                serde_json::json!("Resolve changed sources: wh set-sources --changed-only");
        } else {
            result["next_command"] = serde_json::json!("No drift detected. Rules are current.");
        }
        result["drift"] = drift;
    } else {
        result["next_command"] = serde_json::json!("Resolve docs: wh doctor");
    }

    Ok(result)
}

/// Format detect-deps result as a human-readable summary with scoped package grouping.
pub fn format_human_output(result: &Value) -> String {
    use std::collections::BTreeMap;
    let mut lines = Vec::new();

    // Monorepo info
    let is_monorepo = result
        .get("discovery")
        .and_then(|d| d.get("monorepo"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_monorepo {
        let ws_count = result
            .get("discovery")
            .and_then(|d| d.get("workspaces"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        lines.push(format!("Monorepo: {ws_count} workspaces"));
    }

    // Language counts
    let counts = result.get("counts").and_then(|c| c.get("runtime"));
    if let Some(counts) = counts {
        let total = counts.get("_all").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut lang_parts = Vec::new();
        for (k, v) in counts.as_object().into_iter().flatten() {
            if k != "_all" {
                if let Some(n) = v.as_i64() {
                    lang_parts.push(format!("{n} {k}"));
                }
            }
        }
        lines.push(format!(
            "Dependencies: {total} runtime ({})",
            lang_parts.join(", ")
        ));
    }

    let dev_count = result
        .get("counts")
        .and_then(|c| c.get("dev"))
        .and_then(|c| c.get("_all"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if dev_count > 0 {
        lines.push(format!("Dev dependencies: {dev_count}"));
    }

    // Group deps by scope for display
    if let Some(deps) = result.get("dependencies").and_then(|v| v.as_array()) {
        let runtime: Vec<&Value> = deps
            .iter()
            .filter(|d| !d.get("dev").and_then(|v| v.as_bool()).unwrap_or(false))
            .collect();

        // Group scoped packages
        let mut scope_groups: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        let mut standalone: Vec<(&str, &str)> = Vec::new();

        for dep in &runtime {
            let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("*");
            if let Some(scope) = dep.get("scope").and_then(|v| v.as_str()) {
                let short = name.strip_prefix(&format!("{scope}/")).unwrap_or(name);
                scope_groups.entry(scope.to_string()).or_default().push(short);
            } else {
                standalone.push((name, version));
            }
        }

        lines.push(String::new());
        lines.push("Runtime:".to_string());

        for (scope, packages) in &scope_groups {
            if packages.len() == 1 {
                lines.push(format!("  {scope}/{}", packages[0]));
            } else {
                let preview: Vec<&str> = packages.iter().take(4).copied().collect();
                let suffix = if packages.len() > 4 {
                    format!(", ... +{}", packages.len() - 4)
                } else {
                    String::new()
                };
                lines.push(format!(
                    "  {scope} ({} packages) \u{2014} {}{}",
                    packages.len(),
                    preview.join(", "),
                    suffix,
                ));
            }
        }

        for (name, version) in &standalone {
            lines.push(format!("  {name} {version}"));
        }
    }

    // Next command
    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        lines.push(String::new());
        lines.push(format!("Next: {next}"));
    }

    lines.join("\n")
}

fn is_dev(dep: &Value) -> bool {
    dep.get("dev").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Build a set of "language:name" keys for dependencies that have approved rules.
/// These are protected from stale-entry cleanup.
fn approved_dep_keys(project_dir: &Path) -> HashSet<String> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (rule_files, _) = crate::rules::load_rules_as_json(&rules_dir);
    let mut protected = HashSet::new();
    for rf in &rule_files {
        let has_approved = rf
            .get("rules")
            .and_then(|v| v.as_array())
            .map(|rules| {
                rules
                    .iter()
                    .any(|r| r.get("approved").and_then(|v| v.as_bool()).unwrap_or(false))
            })
            .unwrap_or(false);
        if has_approved {
            if let (Some(lang), Some(name)) = (
                rf.get("language").and_then(|v| v.as_str()),
                rf.get("source_name").and_then(|v| v.as_str()),
            ) {
                protected.insert(format!("{lang}:{name}"));
            }
        }
    }
    protected
}

fn dedup_deps(all_deps: &[Value]) -> Vec<Value> {
    let mut merged: BTreeMap<(String, String, bool), Value> = BTreeMap::new();

    for dep in all_deps {
        let name = dep
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let language = dep
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dev = dep.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
        let source = dep
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("root")
            .to_string();
        let version = dep
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_string();

        let key = (name.clone(), language.clone(), dev);

        if let Some(entry) = merged.get_mut(&key) {
            // Add source
            if let Some(sources) = entry.get_mut("sources").and_then(|v| v.as_array_mut()) {
                let src_val = Value::String(source);
                if !sources.contains(&src_val) {
                    sources.push(src_val);
                }
            }
            // Prefer more specific version
            if entry.get("version").and_then(|v| v.as_str()) == Some("*") && version != "*" {
                entry["version"] = Value::String(version);
            }
        } else {
            merged.insert(
                key,
                serde_json::json!({
                    "name": name,
                    "version": version,
                    "language": language,
                    "dev": dev,
                    "sources": [source],
                }),
            );
        }
    }

    let mut result: Vec<Value> = merged.into_values().collect();
    result.sort_by(|a, b| {
        let a_dev = a.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
        let b_dev = b.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
        a_dev.cmp(&b_dev).then_with(|| {
            let a_name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let b_name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            a_name.cmp(b_name)
        })
    });
    result
}

fn build_counts(
    all: &[Value],
    runtime: &[&Value],
    dev: &[&Value],
    languages: &BTreeSet<String>,
) -> Value {
    let mut total: BTreeMap<String, usize> = BTreeMap::new();
    let mut rt: BTreeMap<String, usize> = BTreeMap::new();
    let mut dv: BTreeMap<String, usize> = BTreeMap::new();

    for lang in languages {
        total.insert(
            lang.clone(),
            all.iter()
                .filter(|d| d.get("language").and_then(|v| v.as_str()) == Some(lang))
                .count(),
        );
        rt.insert(
            lang.clone(),
            runtime
                .iter()
                .filter(|d| d.get("language").and_then(|v| v.as_str()) == Some(lang))
                .count(),
        );
        dv.insert(
            lang.clone(),
            dev.iter()
                .filter(|d| d.get("language").and_then(|v| v.as_str()) == Some(lang))
                .count(),
        );
    }
    total.insert("_all".to_string(), all.len());
    rt.insert("_all".to_string(), runtime.len());
    dv.insert("_all".to_string(), dev.len());

    serde_json::json!({
        "total": total,
        "runtime": rt,
        "dev": dv,
    })
}

fn effective_excluded_list(extra: &[String]) -> Vec<String> {
    let mut all: BTreeSet<String> = walk::SKIP_DIRS.iter().map(|s| s.to_string()).collect();
    all.extend(extra.iter().cloned());
    all.into_iter().collect()
}

fn check_drift(deps: &[Value], project_dir: &Path) -> Value {
    let rules_dir = project_dir.join("whetstone").join("rules");
    if !rules_dir.exists() {
        return serde_json::json!({"changed": [], "count": 0, "checked": 0});
    }

    let name_re = Regex::new(r"(?m)^\s*name:\s*(.+)$").unwrap();
    let ver_re = Regex::new(r#"(?m)^\s*version:\s*['"]?(.+?)['"]?\s*$"#).unwrap();

    let mut stored_versions: HashMap<String, String> = HashMap::new();

    if let Ok(entries) = glob_yaml_files(&rules_dir) {
        for path in entries {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let (Some(name_cap), Some(ver_cap)) =
                    (name_re.captures(&text), ver_re.captures(&text))
                {
                    stored_versions.insert(
                        name_cap[1].trim().to_string(),
                        ver_cap[1].trim().to_string(),
                    );
                }
            }
        }
    }

    let mut changed = Vec::new();
    for dep in deps {
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(stored_ver) = stored_versions.get(name) {
            if stored_ver != version {
                changed.push(serde_json::json!({
                    "name": name,
                    "language": language,
                    "old_version": stored_ver,
                    "new_version": version,
                }));
            }
        }
    }

    let checked = stored_versions.len();
    let count = changed.len();
    serde_json::json!({
        "changed": changed,
        "count": count,
        "checked": checked,
    })
}

fn glob_yaml_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            files.push(entry.path().to_path_buf());
        }
    }
    Ok(files)
}
