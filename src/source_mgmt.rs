//! `wh source` — subscribe to custom rule sources.
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

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::config::{CustomSource, PersonalConfig, WhetstoneConfig};

const VALID_LANGUAGES: &[&str] = &["python", "typescript", "rust", "any"];

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

// ── add ──

pub fn add(project_dir: &Path, opts: AddOptions<'_>) -> Result<Value> {
    validate_url(opts.url)?;
    if let Some(lang) = opts.language {
        if !VALID_LANGUAGES.contains(&lang) {
            return Err(anyhow!(
                "invalid --lang `{lang}`. Must be one of: {}",
                VALID_LANGUAGES.join(", ")
            ));
        }
    }

    let path = target_config_path(project_dir, opts.personal);
    let mut top = read_yaml_mapping_or_empty(&path)?;

    // Walk: top.sources.custom[]. Create intermediate nodes as needed.
    let sources = top
        .entry(ystr("sources"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let sources_map = match sources {
        YamlValue::Mapping(m) => m,
        _ => return Err(anyhow!("{} has a non-mapping `sources` key", path.display())),
    };
    let custom = sources_map
        .entry(ystr("custom"))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()));
    let custom_seq = match custom {
        YamlValue::Sequence(s) => s,
        _ => return Err(anyhow!("{} has a non-sequence `sources.custom`", path.display())),
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
    if let Some(lang) = opts.language {
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
        "next_command": "wh source fetch",
    }))
}

// ── list ──

pub fn list(project_dir: &Path) -> Result<Value> {
    // Load both layers separately so the report shows provenance.
    let project_cfg = WhetstoneConfig::load_project_only(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let personal_cfg = PersonalConfig::load(&paths.personal_config);

    let project_entries: Vec<Value> = project_cfg
        .sources
        .custom
        .iter()
        .map(|s| entry_json(s, "project"))
        .collect();
    let personal_entries: Vec<Value> = personal_cfg
        .sources
        .custom
        .iter()
        .map(|s| entry_json(s, "personal"))
        .collect();

    let total = project_entries.len() + personal_entries.len();

    Ok(json!({
        "status": "ok",
        "total": total,
        "project": project_entries,
        "personal": personal_entries,
    }))
}

pub fn format_list_human(result: &Value) -> String {
    let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    if total == 0 {
        return "No custom sources subscribed. Add one with `wh source add <url>`.\n".to_string();
    }
    let mut out = format!("{total} custom source(s):\n\n");
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
            out.push_str(&format!("  {name}  [{lang} · {kind}]\n    {url}\n"));
        }
        out.push('\n');
    }
    out
}

fn entry_json(s: &CustomSource, layer: &str) -> Value {
    json!({
        "url": s.url,
        "name": s.name,
        "language": s.language,
        "source_kind": s.source_kind,
        "layer": layer,
    })
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
            "wh rule edit <id> or delete the rule file if the source is gone for good"
        } else {
            "wh source list"
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
                    if src == url || src.starts_with(url) {
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

// ── fetch ──

pub fn fetch(project_dir: &Path, target: &str) -> Result<Value> {
    let project_cfg = WhetstoneConfig::load(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let personal_cfg = PersonalConfig::load(&paths.personal_config);

    let mut all: Vec<(&CustomSource, &'static str)> = Vec::new();
    for s in &project_cfg.sources.custom {
        all.push((s, "project"));
    }
    for s in &personal_cfg.sources.custom {
        all.push((s, "personal"));
    }

    let matched: Vec<(&CustomSource, &'static str)> = all
        .into_iter()
        .filter(|(s, _)| s.url == target || s.name.as_deref() == Some(target))
        .collect();

    if matched.is_empty() {
        return Err(anyhow!(
            "source `{target}` not found in either layer. Use `wh source list` to see subscribed sources."
        ));
    }

    let timeout = project_cfg.resolve.timeout_seconds.unwrap_or(15);
    let mut results = Vec::new();
    for (src, layer) in matched {
        let fetched = crate::resolve::resolve_custom_sources(std::slice::from_ref(src), timeout);
        for item in fetched {
            let mut with_layer = item;
            if let Value::Object(ref mut m) = with_layer {
                m.insert("layer".to_string(), Value::String(layer.to_string()));
            }
            results.push(with_layer);
        }
    }

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
    let text = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read {}: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Mapping::new());
    }
    let value: YamlValue = serde_yaml::from_str(&text)
        .map_err(|e| anyhow!("failed to parse {} as YAML: {e}", path.display()))?;
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
    fs::write(path, body).map_err(|e| anyhow!("failed to write {}: {e}", path.display()))
}

fn ystr(s: &str) -> YamlValue {
    YamlValue::String(s.to_string())
}

fn validate_url(url: &str) -> Result<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow!(
            "URL must start with http:// or https:// (got `{url}`)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_url;

    #[test]
    fn url_must_be_http_or_https() {
        assert!(validate_url("https://example.com/llms.txt").is_ok());
        assert!(validate_url("http://example.com").is_ok());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("example.com").is_err());
    }
}
