//! `wh sources` — subscribe to custom rule sources.
//!
//! The resolver and config layers already understand custom sources;
//! this module is the user-facing UX for managing the subscription list
//! (in `whetstone/.personal/config.yaml` by default, or the committed
//! `whetstone/whetstone.yaml` with `--project`).
//!
//! Mutations read the target YAML as a raw mapping, edit the
//! `sources.custom[]` array in place, and write the whole file back. This
//! preserves every other field the user may have configured without
//! needing a schema-typed round-trip.
//!
//! Epic 3E follow-up (`whetstone-gpe`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::config::{PersonalConfig, ResolvedSourceInput, SourcesConfig, WhetstoneConfig};

// ── options ──

pub struct AddOptions<'a> {
    pub url: &'a str,
    pub name: Option<&'a str>,
    pub language: Option<&'a str>,
    pub source_kind: Option<&'a str>,
    pub personal: bool,
}

pub struct RemoveOptions<'a> {
    pub target: &'a str,
    pub personal: bool,
}

pub struct EditOptions<'a> {
    pub target: &'a str,
    pub url: Option<&'a str>,
    pub name: Option<&'a str>,
    pub language: Option<&'a str>,
    pub source_kind: Option<&'a str>,
    pub personal: bool,
}

// ── add ──

pub fn add(project_dir: &Path, opts: AddOptions<'_>) -> Result<Value> {
    validate_source_reference(project_dir, opts.url)?;
    let normalized_language = opts.language.map(normalize_source_language).transpose()?;

    let path = target_config_path(project_dir, opts.personal);
    let mut top = read_yaml_mapping_or_empty(&path)?;

    // Walk: top.sources.custom[]. Create intermediate nodes as needed.
    let sources = top
        .entry(ystr("sources"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let sources_map = match sources {
        YamlValue::Mapping(m) => m,
        _ => {
            return Err(anyhow!(
                "{} has a non-mapping `sources` key",
                path.display()
            ))
        }
    };
    let custom = sources_map
        .entry(ystr("custom"))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()));
    let custom_seq = match custom {
        YamlValue::Sequence(s) => s,
        _ => {
            return Err(anyhow!(
                "{} has a non-sequence `sources.custom`",
                path.display()
            ))
        }
    };

    // Refuse duplicates by URL.
    for entry in custom_seq.iter() {
        let existing_url = entry
            .as_mapping()
            .and_then(|m| m.get(ystr("url")))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if existing_url == opts.url {
            return Err(anyhow!(
                "source already subscribed: {existing_url} (in {})",
                path.display()
            ));
        }
    }

    // Build the new entry. Order fields deterministically so diffs are clean.
    let mut entry = Mapping::new();
    entry.insert(ystr("url"), ystr(opts.url));
    if let Some(name) = opts.name {
        entry.insert(ystr("name"), ystr(name));
    }
    if let Some(lang) = normalized_language {
        entry.insert(ystr("language"), ystr(lang));
    }
    if let Some(kind) = opts.source_kind {
        entry.insert(ystr("source_kind"), ystr(kind));
    }
    custom_seq.push(YamlValue::Mapping(entry));

    write_yaml_mapping(&path, &top)?;

    Ok(json!({
        "status": "ok",
        "wrote": path.display().to_string(),
        "layer": if opts.personal { "personal" } else { "project" },
        "url": opts.url,
        "name": opts.name,
        "next_command": "wh sources verify",
    }))
}

// ── list ──

pub fn list(project_dir: &Path) -> Result<Value> {
    // Load both layers separately so the report shows provenance.
    let project_cfg = WhetstoneConfig::load_project_only(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let personal_cfg = PersonalConfig::load(&paths.personal_config);
    let status_by_key = configured_source_statuses(project_dir);
    let (project_sources, mut project_warnings) =
        collect_source_inputs(project_dir, &project_cfg.sources, "sources_list_project");
    let (personal_sources, mut personal_warnings) =
        collect_source_inputs(project_dir, &personal_cfg.sources, "sources_list_personal");

    let project_entries: Vec<Value> = project_sources
        .iter()
        .map(|s| entry_json(s, "project", status_by_key.get(&source_status_key(s))))
        .collect();
    let personal_entries: Vec<Value> = personal_sources
        .iter()
        .map(|s| entry_json(s, "personal", status_by_key.get(&source_status_key(s))))
        .collect();

    let total = project_entries.len() + personal_entries.len();
    project_warnings.append(&mut personal_warnings);

    Ok(json!({
        "status": "ok",
        "total": total,
        "project": project_entries,
        "personal": personal_entries,
        "warnings": project_warnings,
    }))
}

pub fn format_list_human(result: &Value) -> String {
    let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    if total == 0 {
        return "No configured sources subscribed. Add one with `wh sources add <url>` or define `sources.packs` in config.\n".to_string();
    }
    let mut out = format!("{total} configured source(s):\n\n");
    for layer_key in ["project", "personal"] {
        let empty: Vec<Value> = Vec::new();
        let entries = result
            .get(layer_key)
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        if entries.is_empty() {
            continue;
        }
        out.push_str(&format!("[{layer_key}]\n"));
        for e in entries {
            let name = e
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| e.get("url").and_then(|v| v.as_str()).unwrap_or(""));
            let url = e.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let lang = e.get("language").and_then(|v| v.as_str()).unwrap_or("any");
            let kind = e
                .get("source_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("custom");
            let fetch_state = e
                .get("fetch_state")
                .and_then(|v| v.as_str())
                .unwrap_or("never_fetched");
            let fetched = e
                .get("last_fetched")
                .and_then(|v| v.as_str())
                .unwrap_or("never");
            out.push_str(&format!(
                "  {name}  [{lang} · {kind} · {fetch_state}]\n    {url}\n    last fetched: {fetched}\n"
            ));
            if let Some(origin) = e.get("source_origin").and_then(|v| v.as_str()) {
                out.push_str(&format!("    origin: {origin}\n"));
            }
            if let Some(pack_id) = e.get("pack_id").and_then(|v| v.as_str()) {
                out.push_str(&format!("    pack: {pack_id}\n"));
            }
            if let Some(authority) = e.get("authority").and_then(|v| v.as_str()) {
                out.push_str(&format!("    authority: {authority}\n"));
            }
            if let Some(conf) = e.get("source_confidence").and_then(|v| v.as_str()) {
                out.push_str(&format!("    source confidence: {conf}\n"));
            }
            if let Some(guidance) = e.get("confidence_guidance").and_then(|v| v.as_str()) {
                out.push_str(&format!("    guidance: {guidance}\n"));
            }
        }
        out.push('\n');
    }
    out
}

fn entry_json(s: &ResolvedSourceInput, layer: &str, status: Option<&SourceStatus>) -> Value {
    let last_source_type = status.and_then(|s| s.last_source_type.clone());
    let confidence = last_source_type
        .as_deref()
        .map(source_type_confidence)
        .unwrap_or("unknown");
    json!({
        "url": s.url,
        "name": s.name,
        "language": s.language,
        "source_kind": s.source_kind,
        "source_origin": s.source_origin,
        "source_ref_id": s.source_ref_id,
        "pack_id": s.pack_id,
        "pack_name": s.pack_name,
        "member_id": s.member_id,
        "authority": s.metadata.get("authority"),
        "dep_names": s.metadata.get("dep_names"),
        "upstream_urls": s.metadata.get("upstream_urls"),
        "layer": layer,
        "fetch_state": status.map(|s| s.fetch_state.clone()).unwrap_or_else(|| "never_fetched".to_string()),
        "last_fetched": status.and_then(|s| s.last_fetched.clone()),
        "last_source_type": last_source_type,
        "source_confidence": confidence,
        "confidence_guidance": source_confidence_guidance(confidence),
    })
}

fn source_type_confidence(source_type: &str) -> &'static str {
    match source_type {
        "llms_txt" | "llms_full_txt" | "local_file" => "high",
        "second_brain_page" => "medium",
        "docs_url" | "readme" => "medium",
        _ => "low",
    }
}

fn source_confidence_guidance(confidence: &str) -> &'static str {
    match confidence {
        "high" => "Good extraction source; citations are usually straightforward.",
        "medium" => "Usable source, but review citations carefully before approving rules.",
        "low" => "Low-confidence source; prefer source verification before extraction.",
        _ => "Source has not been fetched yet; run `wh sources verify <name-or-url>`.",
    }
}

fn normalize_source_language(language: &str) -> Result<&'static str> {
    crate::types::normalize_language_or_meta(language, &[crate::types::ANY_LANGUAGE_META])
        .ok_or_else(|| {
            anyhow!(
                "invalid --lang `{language}`. Must be one of: {}",
                crate::types::supported_language_display_list(&[crate::types::ANY_LANGUAGE_META])
            )
        })
}

// ── edit ──

pub fn edit(project_dir: &Path, opts: EditOptions<'_>) -> Result<Value> {
    if opts.url.is_none()
        && opts.name.is_none()
        && opts.language.is_none()
        && opts.source_kind.is_none()
    {
        return Err(anyhow!(
            "nothing to change. Pass at least one of --url, --name, --lang, or --kind"
        ));
    }
    if let Some(url) = opts.url {
        validate_source_reference(project_dir, url)?;
    }
    let normalized_language = opts.language.map(normalize_source_language).transpose()?;

    let path = target_config_path(project_dir, opts.personal);
    if !path.exists() {
        return Err(anyhow!(
            "no config at {}; nothing to edit in the {} layer",
            path.display(),
            if opts.personal { "personal" } else { "project" }
        ));
    }
    let mut top = read_yaml_mapping_or_empty(&path)?;
    let sources = top.get_mut(ystr("sources"));
    let Some(YamlValue::Mapping(sources_map)) = sources else {
        return Err(anyhow!(
            "source `{}` not found in {} (no sources configured)",
            opts.target,
            path.display()
        ));
    };
    let custom = sources_map.get_mut(ystr("custom"));
    let Some(YamlValue::Sequence(custom_seq)) = custom else {
        return Err(anyhow!(
            "source `{}` not found in {} (no custom sources configured)",
            opts.target,
            path.display()
        ));
    };

    let matches: Vec<usize> = custom_seq
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let m = entry.as_mapping()?;
            let url = m
                .get(ystr("url"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let name = m
                .get(ystr("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if url == opts.target || name == opts.target {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if matches.is_empty() {
        return Err(anyhow!(
            "source `{}` not found in {}",
            opts.target,
            path.display()
        ));
    }
    if matches.len() > 1 {
        return Err(anyhow!(
            "source target `{}` matched multiple entries in {}. Re-run with the full URL.",
            opts.target,
            path.display()
        ));
    }

    if let Some(new_url) = opts.url {
        for (idx, entry) in custom_seq.iter().enumerate() {
            if idx == matches[0] {
                continue;
            }
            let existing_url = entry
                .as_mapping()
                .and_then(|m| m.get(ystr("url")))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if existing_url == new_url {
                return Err(anyhow!(
                    "source already subscribed: {existing_url} (in {})",
                    path.display()
                ));
            }
        }
    }

    let Some(YamlValue::Mapping(entry)) = custom_seq.get_mut(matches[0]) else {
        return Err(anyhow!(
            "source `{}` in {} is malformed",
            opts.target,
            path.display()
        ));
    };

    if let Some(url) = opts.url {
        entry.insert(ystr("url"), ystr(url));
    }
    if let Some(name) = opts.name {
        entry.insert(ystr("name"), ystr(name));
    }
    if let Some(lang) = normalized_language {
        entry.insert(ystr("language"), ystr(lang));
    }
    if let Some(kind) = opts.source_kind {
        entry.insert(ystr("source_kind"), ystr(kind));
    }

    write_yaml_mapping(&path, &top)?;

    Ok(json!({
        "status": "ok",
        "wrote": path.display().to_string(),
        "layer": if opts.personal { "personal" } else { "project" },
        "target": opts.target,
        "updated": {
            "url": opts.url,
            "name": opts.name,
            "language": normalized_language,
            "source_kind": opts.source_kind,
        },
        "next_command": "wh sources verify",
    }))
}

// ── remove ──

pub fn remove(project_dir: &Path, opts: RemoveOptions<'_>) -> Result<Value> {
    let path = target_config_path(project_dir, opts.personal);
    if !path.exists() {
        return Err(anyhow!(
            "no config at {}; nothing to remove from the {} layer",
            path.display(),
            if opts.personal { "personal" } else { "project" }
        ));
    }
    let mut top = read_yaml_mapping_or_empty(&path)?;
    let sources = top.get_mut(ystr("sources"));
    let Some(YamlValue::Mapping(sources_map)) = sources else {
        return Err(anyhow!(
            "source `{}` not found in {} (no sources configured)",
            opts.target,
            path.display()
        ));
    };
    let custom = sources_map.get_mut(ystr("custom"));
    let Some(YamlValue::Sequence(custom_seq)) = custom else {
        return Err(anyhow!(
            "source `{}` not found in {} (no custom sources configured)",
            opts.target,
            path.display()
        ));
    };

    let matches: Vec<usize> = custom_seq
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let m = entry.as_mapping()?;
            let url = m
                .get(ystr("url"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let name = m
                .get(ystr("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if url == opts.target || name == opts.target {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if matches.len() > 1 {
        return Err(anyhow!(
            "source target `{}` matched multiple entries in {}. Re-run with the full URL.",
            opts.target,
            path.display()
        ));
    }

    let original_len = custom_seq.len();
    let mut removed_url: Option<String> = None;
    custom_seq.retain(|entry| {
        let m = match entry.as_mapping() {
            Some(m) => m,
            None => return true,
        };
        let url = m
            .get(ystr("url"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let name = m
            .get(ystr("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let matches = url == opts.target || name == opts.target;
        if matches {
            removed_url = Some(url.to_string());
        }
        !matches
    });

    if custom_seq.len() == original_len {
        return Err(anyhow!(
            "source `{}` not found in {}",
            opts.target,
            path.display()
        ));
    }

    write_yaml_mapping(&path, &top)?;

    // Report which approved rules cited this source_url so the agent/user knows
    // what to review. Best-effort string prefix match on source_url.
    let citing_rules = if let Some(url) = &removed_url {
        citing_rule_ids(project_dir, url)
    } else {
        Vec::new()
    };

    Ok(json!({
        "status": "ok",
        "wrote": path.display().to_string(),
        "layer": if opts.personal { "personal" } else { "project" },
        "removed_url": removed_url,
        "citing_rule_ids": citing_rules,
        "next_command": if citing_rules_nonempty_hint(project_dir, &removed_url) {
            "wh rules edit <id> or remove the rule if the source is gone for good"
        } else {
            "wh sources list"
        },
    }))
}

fn citing_rules_nonempty_hint(project_dir: &Path, removed_url: &Option<String>) -> bool {
    removed_url
        .as_ref()
        .map(|u| !citing_rule_ids(project_dir, u).is_empty())
        .unwrap_or(false)
}

fn citing_rule_ids(project_dir: &Path, url: &str) -> Vec<Value> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut out = Vec::new();
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = crate::rules::load_rule_files(dir);
        for lrf in files {
            for r in &lrf.rule_file.rules {
                if let Some(src) = &r.source_url {
                    if source_url_matches_reference(src, url) {
                        out.push(json!({
                            "rule_id": r.id,
                            "file": lrf.file_path,
                        }));
                    }
                }
            }
        }
    }
    out
}

fn source_url_matches_reference(source_url: &str, reference: &str) -> bool {
    if source_url == reference {
        return true;
    }
    source_url
        .strip_prefix(reference)
        .and_then(|suffix| suffix.chars().next())
        .map(|next| matches!(next, '/' | '#' | '?'))
        .unwrap_or(false)
}

// ── fetch ──

pub fn fetch(project_dir: &Path, target: &str) -> Result<Value> {
    let project_cfg = WhetstoneConfig::load_project_only(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let personal_cfg = PersonalConfig::load(&paths.personal_config);

    let mut all: Vec<(ResolvedSourceInput, &'static str)> = Vec::new();
    let (project_sources, _) =
        collect_source_inputs(project_dir, &project_cfg.sources, "sources_fetch_project");
    for s in project_sources {
        all.push((s, "project"));
    }
    let (personal_sources, _) =
        collect_source_inputs(project_dir, &personal_cfg.sources, "sources_fetch_personal");
    for s in personal_sources {
        all.push((s, "personal"));
    }

    let matched: Vec<(ResolvedSourceInput, &'static str)> = all
        .into_iter()
        .filter(|(s, _)| {
            s.url == target
                || s.name.as_deref() == Some(target)
                || s.pack_id.as_deref() == Some(target)
                || s.pack_name.as_deref() == Some(target)
        })
        .collect();

    if matched.is_empty() {
        return Err(anyhow!(
            "source `{target}` not found in either layer. Use `wh sources list` to see subscribed sources."
        ));
    }

    let timeout = project_cfg.resolve.timeout_seconds.unwrap_or(15);
    let ttl = project_cfg
        .resolve
        .cache_ttl_seconds
        .unwrap_or(crate::state::cache::DEFAULT_TTL);
    let mut sm = crate::state::StateManager::new(project_dir);
    sm.ensure_dir();
    sm.load_all();
    let mut results = Vec::new();
    for (src, layer) in matched {
        let fetched =
            crate::resolve::resolve_source_inputs(project_dir, std::slice::from_ref(&src), timeout);
        for item in fetched {
            let mut cache_entry = item.clone();
            if let Value::Object(ref mut m) = cache_entry {
                m.insert("layer".to_string(), Value::String(layer.to_string()));
                m.insert(
                    "fetch_timestamp".to_string(),
                    Value::String(crate::state::now_iso()),
                );
                m.insert("ttl_seconds".to_string(), Value::from(ttl));
            }
            sm.cache.upsert(cache_entry);

            let mut output_item = item;
            if let Value::Object(ref mut m) = output_item {
                let content_bytes = m
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|c| c.len() as u64)
                    .unwrap_or(0);
                m.remove("content");
                m.insert("layer".to_string(), Value::String(layer.to_string()));
                m.insert("content_bytes".to_string(), Value::from(content_bytes));
                m.insert(
                    "fetch_timestamp".to_string(),
                    Value::String(crate::state::now_iso()),
                );
            }
            results.push(output_item);
        }
    }
    sm.cache.save();

    if results.is_empty() {
        return Err(anyhow!(
            "source `{target}` matched a subscription but the resolver returned no content. Check network / URL."
        ));
    }

    Ok(json!({
        "status": "ok",
        "fetched": results.len(),
        "sources": results,
        "next_command": "wh extract",
    }))
}

// ── helpers ──

fn target_config_path(project_dir: &Path, personal: bool) -> PathBuf {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    if personal {
        paths.personal_config
    } else {
        paths.whetstone_dir.join("whetstone.yaml")
    }
}

fn read_yaml_mapping_or_empty(path: &Path) -> Result<Mapping> {
    if !path.exists() {
        return Ok(Mapping::new());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Mapping::new());
    }
    let value: YamlValue = serde_yaml::from_str(&text)
        .with_context(|| format!("failed to parse {} as YAML", path.display()))?;
    match value {
        YamlValue::Mapping(m) => Ok(m),
        _ => Err(anyhow!("{} must be a YAML mapping", path.display())),
    }
}

fn write_yaml_mapping(path: &Path, top: &Mapping) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_yaml::to_string(&YamlValue::Mapping(top.clone()))?;
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))
}

fn ystr(s: &str) -> YamlValue {
    YamlValue::String(s.to_string())
}

fn validate_source_reference(project_dir: &Path, url: &str) -> Result<()> {
    if !url.starts_with("http://")
        && !url.starts_with("https://")
        && !is_repo_relative_path(project_dir, url)
    {
        return Err(anyhow!(
            "source must be an http(s) URL or repo-relative file path (got `{url}`)"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct SourceStatus {
    fetch_state: String,
    last_fetched: Option<String>,
    last_source_type: Option<String>,
}

fn source_entry_is_fresh(entry: &Value) -> bool {
    let Some(fetch_timestamp) = entry
        .get("fetch_timestamp")
        .or_else(|| entry.get("fetched_at"))
        .and_then(|v| v.as_str())
    else {
        return false;
    };
    let ttl = entry
        .get("ttl_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(crate::state::cache::DEFAULT_TTL);
    let Some(parsed) = chrono::DateTime::parse_from_rfc3339(fetch_timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
    else {
        return false;
    };
    (chrono::Utc::now() - parsed).num_seconds() < ttl as i64
}

fn source_status_key(source: &ResolvedSourceInput) -> String {
    if source.source_origin == "custom" {
        let language = source.language.as_deref().unwrap_or("any");
        let name = source.name.as_deref().unwrap_or(source.url.as_str());
        format!("{language}:{name}:custom")
    } else {
        source.source_ref_id.clone()
    }
}

fn collect_source_inputs(
    project_dir: &Path,
    sources: &SourcesConfig,
    trigger: &str,
) -> (Vec<ResolvedSourceInput>, Vec<String>) {
    let mut inputs = sources.resolved_inputs();
    let second_brain = crate::second_brain::build(project_dir, &sources.vaults, trigger);
    inputs.extend(second_brain.inputs);
    (inputs, second_brain.warnings)
}

fn configured_source_statuses(project_dir: &Path) -> BTreeMap<String, SourceStatus> {
    let mut sm = crate::state::StateManager::new(project_dir);
    sm.load_all();
    let mut out = BTreeMap::new();
    for entry in sm.cache.all_entries() {
        let version = entry.get("version").and_then(|v| v.as_str()).unwrap_or("");
        if version != "custom" {
            continue;
        }
        let language = entry
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("any");
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let key = entry
            .get("source_ref")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("{language}:{name}:custom"));
        let status = SourceStatus {
            fetch_state: if source_entry_is_fresh(&entry) {
                "fresh".to_string()
            } else {
                "stale".to_string()
            },
            last_fetched: entry
                .get("fetch_timestamp")
                .or_else(|| entry.get("fetched_at"))
                .and_then(|v| v.as_str())
                .map(String::from),
            last_source_type: entry
                .get("source_type")
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        out.insert(key.clone(), status.clone());
        if entry
            .get("source_origin")
            .and_then(|v| v.as_str())
            .unwrap_or("custom")
            == "custom"
        {
            out.insert(format!("{language}:{name}:custom"), status);
        }
    }
    out
}

fn is_repo_relative_path(project_dir: &Path, input: &str) -> bool {
    if input.trim().is_empty() {
        return false;
    }
    let path = Path::new(input);
    if path.is_absolute() || input.contains("://") {
        return false;
    }
    project_dir.join(path).exists()
}

#[cfg(test)]
mod tests {
    use super::{list, validate_source_reference};
    use std::path::Path;

    #[test]
    fn url_must_be_http_or_https() {
        let tmp = std::env::temp_dir().join(format!("wh_source_ref_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("docs")).unwrap();
        std::fs::create_dir_all(tmp.join("notes")).unwrap();
        std::fs::write(tmp.join("docs/guide.md"), "guide").unwrap();
        std::fs::write(tmp.join("notes/source.txt"), "source").unwrap();
        std::fs::write(tmp.join("llms.txt"), "llms").unwrap();

        assert!(validate_source_reference(Path::new(&tmp), "https://example.com/llms.txt").is_ok());
        assert!(validate_source_reference(Path::new(&tmp), "http://example.com").is_ok());
        assert!(validate_source_reference(Path::new(&tmp), "docs/guide.md").is_ok());
        assert!(validate_source_reference(Path::new(&tmp), "./notes/source.txt").is_ok());
        assert!(validate_source_reference(Path::new(&tmp), "llms.txt").is_ok());
        assert!(validate_source_reference(Path::new(&tmp), "ftp://example.com").is_err());
        assert!(validate_source_reference(Path::new(&tmp), "example.com").is_err());
        assert!(validate_source_reference(Path::new(&tmp), "/tmp/source.md").is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_surfaces_custom_source_fetch_state() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_source_state_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();
        std::fs::write(
            tmp.join("whetstone/whetstone.yaml"),
            "sources:\n  custom:\n    - url: https://example.com/style\n      name: team-style\n      language: python\n      source_kind: team_guide\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("whetstone/.state/source-cache.json"),
            r#"{
  "version": 1,
  "entries": {
    "python:team-style:custom": {
      "name": "team-style",
      "language": "python",
      "version": "custom",
      "source_type": "llms_txt",
      "fetch_timestamp": "2099-01-01T00:00:00Z"
    }
  }
}"#,
        )
        .unwrap();

        let result = list(&tmp).unwrap();
        let project = result["project"].as_array().unwrap();
        assert_eq!(project.len(), 1);
        assert_eq!(project[0]["fetch_state"], "fresh");
        assert_eq!(project[0]["last_source_type"], "llms_txt");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_surfaces_pack_member_sources() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_source_pack_state_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();
        std::fs::write(
            tmp.join("whetstone/whetstone.yaml"),
            "sources:\n  packs:\n    - id: frontend-guidelines\n      name: frontend-guidelines\n      language: javascript\n      source_kind: team_guide\n      members:\n        - url: https://example.com/frontend/js\n          name: js-guide\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("whetstone/.state/source-cache.json"),
            r#"{
  "version": 1,
  "entries": {
    "source:pack:frontend-guidelines:member-1-js-guide": {
      "name": "js-guide",
      "language": "javascript",
      "version": "custom",
      "source_origin": "pack_member",
      "source_ref": {
        "kind": "pack_member",
        "id": "pack:frontend-guidelines:member-1-js-guide",
        "pack_id": "frontend-guidelines",
        "pack_name": "frontend-guidelines",
        "member_id": "member-1-js-guide"
      },
      "source_type": "llms_txt",
      "fetch_timestamp": "2099-01-01T00:00:00Z"
    }
  }
}"#,
        )
        .unwrap();

        let result = list(&tmp).unwrap();
        let project = result["project"].as_array().unwrap();
        assert_eq!(project.len(), 1);
        assert_eq!(project[0]["source_origin"], "pack_member");
        assert_eq!(project[0]["pack_id"], "frontend-guidelines");
        assert_eq!(project[0]["fetch_state"], "fresh");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
