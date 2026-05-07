use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{
    CheckConfig, ConfigPackRef, DiscoveryConfig, ExtractionConfig, GenerateConfig, GlobalConfig,
    ResolveConfig, SourcesConfig,
};
use crate::resolve;
use crate::rules::{ApprovedExample, ApprovedRule, ApprovedSignal, Rule};
use crate::state::{atomic_write, load_json, now_iso};

const PACK_CACHE_VERSION: i64 = 1;
const VALID_SCOPES: &[&str] = &["org", "team", "project", "personal"];
const VALID_PACK_LANGUAGES: &[&str] = &["python", "typescript", "rust"];

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PackMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PackRuleOverride {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct RulePackFile {
    #[serde(rename = "apiVersion", default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub metadata: PackMetadata,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub extraction: ExtractionConfig,
    #[serde(default)]
    pub resolve: ResolveConfig,
    #[serde(default)]
    pub check: CheckConfig,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub overrides: Vec<PackRuleOverride>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfigPack {
    pub scope: String,
    pub ref_spec: String,
    pub resolved_ref: String,
    pub source_kind: String,
    pub cache_status: String,
    pub content_hash: String,
    pub fetched_at: String,
    pub metadata: PackMetadata,
    pub language: Option<String>,
    pub pack: RulePackFile,
}

#[derive(Debug, Default, Clone)]
pub struct ResolvedPackSet {
    pub packs: Vec<ResolvedConfigPack>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug)]
enum PackTarget {
    Path {
        path: PathBuf,
        resolved_ref: String,
        source_kind: &'static str,
    },
    Remote {
        url: String,
        resolved_ref: String,
        source_kind: &'static str,
    },
}

struct FetchOutcome {
    content: String,
    content_hash: String,
    fetched_at: String,
    cache_status: String,
    warning: Option<String>,
}

pub fn resolve_project_packs(
    project_dir: &Path,
    refs: &[ConfigPackRef],
    global_cfg: &GlobalConfig,
    project_timeout: Option<u64>,
    project_ttl: Option<u64>,
) -> ResolvedPackSet {
    let timeout = project_timeout
        .or(global_cfg.resolve.timeout_seconds)
        .unwrap_or(15);
    let ttl = project_ttl
        .or(global_cfg.resolve.cache_ttl_seconds)
        .unwrap_or(crate::state::cache::DEFAULT_TTL);

    let cache_path = pack_cache_path(project_dir);
    let mut cache = load_pack_cache(&cache_path);
    let mut out = ResolvedPackSet::default();

    for pack_ref in refs {
        let scope = pack_ref
            .scope
            .clone()
            .unwrap_or_else(|| "project".to_string());
        if !VALID_SCOPES.contains(&scope.as_str()) {
            out.warnings.push(format!(
                "pack ref `{}` uses unknown scope `{scope}`; expected one of {:?}",
                pack_ref.ref_, VALID_SCOPES
            ));
        }

        let target = match resolve_pack_target(project_dir, &pack_ref.ref_) {
            Ok(target) => target,
            Err(e) => {
                out.errors.push(format!("{} ({})", e, pack_ref.ref_));
                continue;
            }
        };

        let fetched =
            match fetch_pack_content(&mut cache, &scope, &pack_ref.ref_, &target, timeout, ttl) {
                Ok(fetched) => fetched,
                Err(e) => {
                    out.errors
                        .push(format!("failed to load pack `{}`: {e}", pack_ref.ref_));
                    continue;
                }
            };
        if let Some(w) = fetched.warning.clone() {
            out.warnings.push(w);
        }

        let pack: RulePackFile = match serde_yaml::from_str(&fetched.content) {
            Ok(pack) => pack,
            Err(e) => {
                out.errors
                    .push(format!("pack `{}` parse error: {e}", pack_ref.ref_));
                continue;
            }
        };

        validate_pack_shape(&pack, &scope, &pack_ref.ref_, &mut out);

        let (resolved_ref, source_kind) = match &target {
            PackTarget::Path {
                resolved_ref,
                source_kind,
                ..
            }
            | PackTarget::Remote {
                resolved_ref,
                source_kind,
                ..
            } => (resolved_ref.clone(), (*source_kind).to_string()),
        };

        update_cache_metadata(
            &mut cache,
            &scope,
            &pack_ref.ref_,
            &resolved_ref,
            &pack,
            &fetched,
        );

        out.packs.push(ResolvedConfigPack {
            scope,
            ref_spec: pack_ref.ref_.clone(),
            resolved_ref,
            source_kind,
            cache_status: fetched.cache_status,
            content_hash: fetched.content_hash,
            fetched_at: fetched.fetched_at,
            metadata: pack.metadata.clone(),
            language: pack.language.clone(),
            pack,
        });
    }

    save_pack_cache(&cache_path, &cache);
    out
}

pub fn merge_pack_rules(
    packs: &[ResolvedConfigPack],
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    #[derive(Clone)]
    struct AccumRule {
        rule: ApprovedRule,
        order: usize,
    }

    let mut warnings = Vec::new();
    let mut merged: BTreeMap<String, AccumRule> = BTreeMap::new();
    let mut next_order = 0usize;

    for pack in packs {
        for denied in &pack.pack.deny {
            merged.remove(denied);
        }

        for ov in &pack.pack.overrides {
            if ov.id.trim().is_empty() {
                warnings.push(format!(
                    "pack `{}` has an override with an empty id; ignoring it",
                    pack_display_name(pack)
                ));
                continue;
            }
            match merged.get_mut(&ov.id) {
                Some(existing) => {
                    if let Some(sev) = &ov.severity {
                        existing.rule.severity = sev.clone();
                    }
                    if let Some(conf) = &ov.confidence {
                        existing.rule.confidence = conf.clone();
                    }
                    if let Some(desc) = &ov.description {
                        existing.rule.description = desc.clone();
                    }
                    if let Some(url) = &ov.source_url {
                        existing.rule.source_url = url.clone();
                    }
                }
                None => warnings.push(format!(
                    "pack `{}` overrides `{}` but no broader imported rule with that id exists",
                    pack_display_name(pack),
                    ov.id
                )),
            }
        }

        let pack_language = match pack_rule_language(pack) {
            Ok(Some(lang)) => lang,
            Ok(None) => continue,
            Err(e) => {
                warnings.push(e);
                continue;
            }
        };

        if let Some(filter) = lang_filter {
            if pack_language != filter {
                continue;
            }
        }

        for rule in &pack.pack.rules {
            if !rule.approved {
                continue;
            }

            let approved = approved_from_pack_rule(rule, pack, &pack_language);
            if merged.contains_key(&approved.id) {
                warnings.push(format!(
                    "pack `{}` redefines imported rule `{}`; later imported definition wins",
                    pack_display_name(pack),
                    approved.id
                ));
            }
            merged.insert(
                approved.id.clone(),
                AccumRule {
                    rule: approved,
                    order: next_order,
                },
            );
            next_order += 1;
        }
    }

    let mut rules = merged.into_values().collect::<Vec<_>>();
    rules.sort_by_key(|r| r.order);
    (rules.into_iter().map(|r| r.rule).collect(), warnings)
}

pub fn packs_to_json(packs: &[ResolvedConfigPack]) -> Value {
    Value::Array(
        packs
            .iter()
            .map(|pack| {
                json!({
                    "scope": pack.scope,
                    "name": pack.metadata.name,
                    "version": pack.metadata.version,
                    "owner": pack.metadata.owner,
                    "language": pack.language,
                    "ref": pack.ref_spec,
                    "resolved_ref": pack.resolved_ref,
                    "source_kind": pack.source_kind,
                    "cache_status": pack.cache_status,
                    "content_hash": pack.content_hash,
                    "fetched_at": pack.fetched_at,
                    "rules_count": pack.pack.rules.iter().filter(|r| r.approved).count(),
                    "overrides_count": pack.pack.overrides.len(),
                    "deny_count": pack.pack.deny.len(),
                })
            })
            .collect(),
    )
}

pub fn pack_display_name(pack: &ResolvedConfigPack) -> String {
    pack.metadata
        .name
        .clone()
        .unwrap_or_else(|| pack.ref_spec.clone())
}

fn validate_pack_shape(
    pack: &RulePackFile,
    scope: &str,
    ref_spec: &str,
    out: &mut ResolvedPackSet,
) {
    if let Some(api) = &pack.api_version {
        if api != "whetstone/v1alpha1" {
            out.warnings.push(format!(
                "pack `{ref_spec}` declares apiVersion `{api}`; expected `whetstone/v1alpha1`"
            ));
        }
    }
    if let Some(kind) = &pack.kind {
        if kind != "RulePack" {
            out.warnings.push(format!(
                "pack `{ref_spec}` declares kind `{kind}`; expected `RulePack`"
            ));
        }
    }
    if let Some(meta_scope) = &pack.metadata.scope {
        if meta_scope != scope {
            out.warnings.push(format!(
                "pack `{ref_spec}` metadata.scope is `{meta_scope}` but imported as scope `{scope}`"
            ));
        }
    }
    if !pack.rules.is_empty() {
        match pack.language.as_deref() {
            Some(lang) if VALID_PACK_LANGUAGES.contains(&lang) => {}
            Some(lang) => out.errors.push(format!(
                "pack `{ref_spec}` declares unsupported language `{lang}`; expected one of {:?}",
                VALID_PACK_LANGUAGES
            )),
            None => out.errors.push(format!(
                "pack `{ref_spec}` contains rules but has no top-level `language`"
            )),
        }
    }
}

fn approved_from_pack_rule(rule: &Rule, pack: &ResolvedConfigPack, language: &str) -> ApprovedRule {
    ApprovedRule {
        id: rule.id.clone(),
        severity: rule.severity.clone().unwrap_or_default(),
        confidence: rule.confidence.clone().unwrap_or_default(),
        category: rule.category.clone().unwrap_or_default(),
        description: rule.description.clone().unwrap_or_default(),
        source_url: rule.source_url.clone().unwrap_or_default(),
        source_name: rule
            .id
            .split('.')
            .next()
            .unwrap_or_else(|| pack.metadata.name.as_deref().unwrap_or("pack"))
            .to_string(),
        language: language.to_string(),
        signals: rule
            .signals
            .iter()
            .map(|s| ApprovedSignal {
                id: s.id.clone().unwrap_or_default(),
                strategy: s.strategy.clone(),
                description: s.description.clone().unwrap_or_default(),
                weight: s.weight.clone().unwrap_or_default(),
                match_pattern: s.match_pattern.clone(),
                ast_query: s.ast_query.clone(),
                ast_scope: s.ast_scope.clone(),
            })
            .collect(),
        formatter: rule
            .formatter
            .as_ref()
            .map(|f| crate::rules::ApprovedFormatterDirective {
                tool: f.tool.clone(),
                options: f.options.clone(),
            }),
        golden_examples: rule
            .golden_examples
            .iter()
            .map(|e| ApprovedExample {
                code: e.code.clone(),
                verdict: e.verdict.clone(),
                reason: e.reason.clone().unwrap_or_default(),
                language: e.language.clone(),
            })
            .collect(),
        deterministic_pass_threshold: rule.deterministic_pass_threshold,
        deterministic_fail_threshold: rule.deterministic_fail_threshold,
    }
}

fn pack_rule_language(pack: &ResolvedConfigPack) -> Result<Option<String>, String> {
    if pack.pack.rules.is_empty() {
        return Ok(None);
    }
    let Some(language) = pack.language.as_deref() else {
        return Err(format!(
            "pack `{}` contains rules but has no top-level language",
            pack_display_name(pack)
        ));
    };
    if !VALID_PACK_LANGUAGES.contains(&language) {
        return Err(format!(
            "pack `{}` declares unsupported rule language `{language}`",
            pack_display_name(pack)
        ));
    }
    Ok(Some(language.to_string()))
}

fn pack_cache_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("whetstone")
        .join(".state")
        .join("config-pack-cache.json")
}

fn load_pack_cache(path: &Path) -> Value {
    let raw = load_json(path);
    if raw.get("version").and_then(|v| v.as_i64()) == Some(PACK_CACHE_VERSION) {
        raw
    } else {
        json!({"version": PACK_CACHE_VERSION, "entries": {}})
    }
}

fn save_pack_cache(path: &Path, cache: &Value) {
    let mut data = cache.clone();
    data["version"] = Value::from(PACK_CACHE_VERSION);
    data["updated_at"] = Value::from(now_iso());
    atomic_write(path, &data);
}

fn cache_entries_mut(cache: &mut Value) -> &mut serde_json::Map<String, Value> {
    if cache.get("entries").is_none() {
        cache["entries"] = Value::Object(Default::default());
    }
    cache["entries"].as_object_mut().unwrap()
}

fn cache_key(scope: &str, ref_spec: &str) -> String {
    format!("{scope}:{ref_spec}")
}

fn resolve_pack_target(project_dir: &Path, ref_spec: &str) -> Result<PackTarget> {
    if let Some(rest) = ref_spec.strip_prefix("path:") {
        let raw = PathBuf::from(rest);
        let path = if raw.is_absolute() {
            raw
        } else {
            project_dir.join(raw)
        };
        return Ok(PackTarget::Path {
            resolved_ref: path.display().to_string(),
            path,
            source_kind: "path",
        });
    }

    if let Some(rest) = ref_spec.strip_prefix("file://") {
        let path = PathBuf::from(rest);
        return Ok(PackTarget::Path {
            resolved_ref: path.display().to_string(),
            path,
            source_kind: "file",
        });
    }

    if let Some(rest) = ref_spec.strip_prefix("github://") {
        let (repo, path_and_ref) = rest.split_once("//").ok_or_else(|| {
            anyhow!("invalid github ref `{ref_spec}` — expected github://owner/repo//path@ref")
        })?;
        let (path, git_ref) = path_and_ref
            .rsplit_once('@')
            .ok_or_else(|| anyhow!("invalid github ref `{ref_spec}` — missing @ref"))?;
        let url = format!("https://raw.githubusercontent.com/{repo}/{git_ref}/{path}");
        return Ok(PackTarget::Remote {
            url: url.clone(),
            resolved_ref: url,
            source_kind: "github",
        });
    }

    Err(anyhow!(
        "unsupported pack ref `{ref_spec}` — expected path:, file://, or github://"
    ))
}

fn fetch_pack_content(
    cache: &mut Value,
    scope: &str,
    ref_spec: &str,
    target: &PackTarget,
    timeout: u64,
    ttl: u64,
) -> Result<FetchOutcome> {
    match target {
        PackTarget::Path { path, .. } => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
            let fetched_at = now_iso();
            let content_hash = resolve::content_hash(&content);
            cache_entries_mut(cache).insert(
                cache_key(scope, ref_spec),
                json!({
                    "scope": scope,
                    "ref": ref_spec,
                    "resolved_ref": path.display().to_string(),
                    "source_kind": "path",
                    "content": content,
                    "content_hash": content_hash,
                    "fetched_at": fetched_at,
                }),
            );
            Ok(FetchOutcome {
                content: std::fs::read_to_string(path)
                    .map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?,
                content_hash,
                fetched_at,
                cache_status: "direct".to_string(),
                warning: None,
            })
        }
        PackTarget::Remote {
            url,
            resolved_ref,
            source_kind,
        } => {
            let key = cache_key(scope, ref_spec);
            let cached = cache.get("entries").and_then(|v| v.get(&key)).cloned();

            if let Some(entry) = cached.as_ref() {
                if cache_entry_fresh(entry, ttl)
                    && entry.get("resolved_ref").and_then(|v| v.as_str()) == Some(resolved_ref)
                {
                    if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
                        return Ok(FetchOutcome {
                            content: content.to_string(),
                            content_hash: entry
                                .get("content_hash")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            fetched_at: entry
                                .get("fetched_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            cache_status: "hit".to_string(),
                            warning: None,
                        });
                    }
                }
            }

            if let Some(content) = resolve::http::http_get(url, timeout) {
                let fetched_at = now_iso();
                let content_hash = resolve::content_hash(&content);
                cache_entries_mut(cache).insert(
                    key,
                    json!({
                        "scope": scope,
                        "ref": ref_spec,
                        "resolved_ref": resolved_ref,
                        "source_kind": source_kind,
                        "content": content,
                        "content_hash": content_hash,
                        "fetched_at": fetched_at,
                    }),
                );
                return Ok(FetchOutcome {
                    content,
                    content_hash,
                    fetched_at,
                    cache_status: "miss".to_string(),
                    warning: None,
                });
            }

            if let Some(entry) = cached.as_ref() {
                if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
                    return Ok(FetchOutcome {
                        content: content.to_string(),
                        content_hash: entry
                            .get("content_hash")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        fetched_at: entry
                            .get("fetched_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        cache_status: "stale".to_string(),
                        warning: Some(format!(
                            "remote pack `{ref_spec}` could not be refreshed; using cached content from {}",
                            entry
                                .get("fetched_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or("an unknown time")
                        )),
                    });
                }
            }

            Err(anyhow!("could not fetch remote pack from {url}"))
        }
    }
}

fn update_cache_metadata(
    cache: &mut Value,
    scope: &str,
    ref_spec: &str,
    resolved_ref: &str,
    pack: &RulePackFile,
    fetched: &FetchOutcome,
) {
    let key = cache_key(scope, ref_spec);
    if let Some(entry) = cache_entries_mut(cache).get_mut(&key) {
        entry["resolved_ref"] = Value::from(resolved_ref.to_string());
        entry["metadata"] = json!({
            "name": pack.metadata.name,
            "version": pack.metadata.version,
            "scope": pack.metadata.scope,
            "owner": pack.metadata.owner,
        });
        entry["language"] = Value::from(pack.language.clone());
        entry["rules_count"] = Value::from(pack.rules.iter().filter(|r| r.approved).count() as u64);
        entry["overrides_count"] = Value::from(pack.overrides.len() as u64);
        entry["deny_count"] = Value::from(pack.deny.len() as u64);
        entry["content_hash"] = Value::from(fetched.content_hash.clone());
        entry["fetched_at"] = Value::from(fetched.fetched_at.clone());
    }
}

fn cache_entry_fresh(entry: &Value, ttl: u64) -> bool {
    let Some(ts) = entry.get("fetched_at").and_then(|v| v.as_str()) else {
        return false;
    };
    parse_age_seconds(ts)
        .map(|age| age < ttl as f64)
        .unwrap_or(false)
}

fn parse_age_seconds(ts: &str) -> Option<f64> {
    let parsed: DateTime<Utc> = ts.parse().ok().or_else(|| {
        DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })?;
    Some((Utc::now() - parsed).num_seconds() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_ref_converts_to_raw_url() {
        let td = tempfile::tempdir().unwrap();
        let target = resolve_pack_target(
            td.path(),
            "github://acme/whetstone-config//packs/org/base.yaml@main",
        )
        .unwrap();
        match target {
            PackTarget::Remote { url, .. } => assert_eq!(
                url,
                "https://raw.githubusercontent.com/acme/whetstone-config/main/packs/org/base.yaml"
            ),
            _ => panic!("expected remote target"),
        }
    }

    #[test]
    fn merge_pack_rules_applies_overrides_and_denies() {
        let base = ResolvedConfigPack {
            scope: "org".into(),
            ref_spec: "path:./packs/org.yaml".into(),
            resolved_ref: "/tmp/org.yaml".into(),
            source_kind: "path".into(),
            cache_status: "direct".into(),
            content_hash: "sha256:1".into(),
            fetched_at: now_iso(),
            metadata: PackMetadata {
                name: Some("acme.base".into()),
                ..PackMetadata::default()
            },
            language: Some("python".into()),
            pack: RulePackFile {
                language: Some("python".into()),
                rules: vec![Rule {
                    id: "fastapi.async-routes".into(),
                    severity: Some("should".into()),
                    confidence: Some("high".into()),
                    category: Some("convention".into()),
                    description: Some("Route handlers should be async.".into()),
                    source_url: Some("https://example.com".into()),
                    source_quote: None,
                    approved: true,
                    status: Some("approved".into()),
                    deterministic_pass_threshold: None,
                    deterministic_fail_threshold: None,
                    signals: Vec::new(),
                    formatter: None,
                    golden_examples: Vec::new(),
                }],
                ..RulePackFile::default()
            },
        };

        let override_pack = ResolvedConfigPack {
            scope: "team".into(),
            ref_spec: "path:./packs/team.yaml".into(),
            resolved_ref: "/tmp/team.yaml".into(),
            source_kind: "path".into(),
            cache_status: "direct".into(),
            content_hash: "sha256:2".into(),
            fetched_at: now_iso(),
            metadata: PackMetadata {
                name: Some("acme.team".into()),
                ..PackMetadata::default()
            },
            language: Some("python".into()),
            pack: RulePackFile {
                language: Some("python".into()),
                overrides: vec![PackRuleOverride {
                    id: "fastapi.async-routes".into(),
                    severity: Some("must".into()),
                    ..PackRuleOverride::default()
                }],
                ..RulePackFile::default()
            },
        };

        let (rules, warnings) = merge_pack_rules(&[base, override_pack], Some("python"));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].severity, "must");
    }
}
