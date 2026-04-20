//! `wh rules query` — JIT rule lookup for agents.
//!
//! The agent passes a file path (or language / dep / severity filters) and
//! gets back only the approved rules that apply. This is the token-efficient
//! alternative to loading the entire `AGENTS.md` bootstrap during a turn.
//!
//! Part of Epic 3E (whetstone-n34), theme A — Architecture / JIT consumption.

use std::path::{Path, PathBuf};

use crate::layers::{resolve_merged, Layer, LayeredRule};
use crate::rules::ApprovedRule;
use serde_json::{json, Value};

/// Filters accepted by the query. Empty filters match everything.
pub struct Filters<'a> {
    pub file: Option<&'a Path>,
    pub lang: Option<&'a str>,
    pub dep: Option<&'a str>,
    pub severity: Option<&'a str>,
    pub layer_filter: LayerFilter,
}

#[derive(Clone, Copy)]
pub enum LayerFilter {
    All,
    PersonalOnly,
    ProjectOnly,
}

/// How much detail to include in each rule's JSON payload.
#[derive(Clone, Copy)]
pub enum Detail {
    /// Summary: id, severity, confidence, category, description, source_url,
    /// source_name, language, match_patterns, layer. Cheapest for agents.
    Summary,
    /// Summary plus full signals + golden examples. For humans / debugging.
    Full,
}

/// Result container: filtered rules + total count + warnings from loading.
pub struct QueryResult {
    pub rules: Vec<LayeredRule>,
    pub total: usize,
    pub warnings: Vec<String>,
}

/// Infer language from a source file path's extension.
pub fn detect_language_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "rs" => Some("rust"),
        _ => None,
    }
}

/// Run the query. Returns matching LayeredRules in load order.
pub fn query(project_dir: &Path, filters: &Filters) -> QueryResult {
    // Resolve language filter: explicit --lang wins; otherwise infer from --file.
    let lang_from_file = filters.file.and_then(detect_language_from_path);
    let effective_lang: Option<&str> = filters.lang.or(lang_from_file);

    let include_personal = !matches!(filters.layer_filter, LayerFilter::ProjectOnly);
    let resolved = resolve_merged(project_dir, effective_lang, true, include_personal, false);

    let mut matching: Vec<LayeredRule> = resolved
        .merged
        .into_iter()
        .filter(|lr| layer_matches(lr.layer, filters.layer_filter))
        .filter(|lr| dep_matches(&lr.rule, filters.dep))
        .filter(|lr| severity_matches(&lr.rule, filters.severity))
        .collect();

    // Deterministic order for agent consumption: severity (must > should > may),
    // then id alphabetically.
    matching.sort_by(|a, b| {
        severity_rank(&a.rule.severity)
            .cmp(&severity_rank(&b.rule.severity))
            .then_with(|| a.rule.id.cmp(&b.rule.id))
    });

    let total = matching.len();
    QueryResult {
        rules: matching,
        total,
        warnings: resolved.warnings,
    }
}

/// Serialize a QueryResult into JSON matching the schema documented in
/// `references/workflow-matrix.md`.
pub fn to_json(result: &QueryResult, detail: Detail, echo_filters: Value) -> Value {
    json!({
        "total": result.total,
        "filters": echo_filters,
        "warnings": result.warnings,
        "rules": result
            .rules
            .iter()
            .map(|lr| rule_to_json(lr, detail))
            .collect::<Vec<_>>(),
    })
}

/// Format the rules as a compact human-readable block (non-JSON).
pub fn to_human(result: &QueryResult, detail: Detail) -> String {
    if result.rules.is_empty() {
        return "No approved rules match the given filters.\n".to_string();
    }
    let mut out = String::new();
    out.push_str(&format!("{} rule(s) match:\n\n", result.total));
    for lr in &result.rules {
        out.push_str(&format!(
            "  [{}] {} — {}\n    layer: {}  lang: {}  source: {}\n    {}\n",
            lr.rule.severity.to_uppercase(),
            lr.rule.id,
            trim_one_line(&lr.rule.description, 100),
            lr.layer.as_str(),
            lr.rule.language,
            lr.rule.source_name,
            lr.rule.source_url,
        ));
        if matches!(detail, Detail::Full) {
            for sig in &lr.rule.signals {
                out.push_str(&format!(
                    "      signal [{}] {}\n",
                    sig.strategy,
                    sig.description.chars().take(80).collect::<String>()
                ));
            }
        }
        out.push('\n');
    }
    for w in &result.warnings {
        out.push_str(&format!("⚠ {w}\n"));
    }
    out
}

// --- helpers ---

fn rule_to_json(lr: &LayeredRule, detail: Detail) -> Value {
    let r = &lr.rule;
    let match_patterns: Vec<String> = r
        .signals
        .iter()
        .filter_map(|s| s.match_pattern.clone())
        .collect();

    let mut v = json!({
        "id": r.id,
        "severity": r.severity,
        "confidence": r.confidence,
        "category": r.category,
        "description": r.description,
        "source_url": r.source_url,
        "source_name": r.source_name,
        "language": r.language,
        "layer": lr.layer.as_str(),
        "match_patterns": match_patterns,
    });

    if matches!(detail, Detail::Full) {
        v["signals"] = r
            .signals
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "strategy": s.strategy,
                    "description": s.description,
                    "weight": s.weight,
                    "match": s.match_pattern,
                    "ast_query": s.ast_query,
                    "ast_scope": s.ast_scope,
                })
            })
            .collect::<Vec<_>>()
            .into();
        v["golden_examples"] = r
            .golden_examples
            .iter()
            .map(|e| {
                json!({
                    "code": e.code,
                    "verdict": e.verdict,
                    "reason": e.reason,
                    "language": e.language,
                })
            })
            .collect::<Vec<_>>()
            .into();
    }

    v
}

fn layer_matches(layer: Layer, filter: LayerFilter) -> bool {
    matches!(
        (layer, filter),
        (_, LayerFilter::All)
            | (Layer::Personal, LayerFilter::PersonalOnly)
            | (Layer::Project, LayerFilter::ProjectOnly)
    )
}

fn dep_matches(rule: &ApprovedRule, dep: Option<&str>) -> bool {
    let Some(target) = dep else {
        return true;
    };
    let needle = target.to_lowercase();
    rule.source_name.to_lowercase() == needle
        || rule.id.to_lowercase().starts_with(&format!("{needle}."))
}

fn severity_matches(rule: &ApprovedRule, severity: Option<&str>) -> bool {
    let Some(target) = severity else {
        return true;
    };
    rule.severity.eq_ignore_ascii_case(target)
}

fn severity_rank(sev: &str) -> u8 {
    match sev {
        "must" => 0,
        "should" => 1,
        "may" => 2,
        _ => 3,
    }
}

fn trim_one_line(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if flat.chars().count() <= max {
        flat
    } else {
        let truncated: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Build a JSON representation of the filters for echoing back in the result.
/// Useful for agents that want to verify what filters were actually applied.
pub fn filters_to_json(
    file: Option<&Path>,
    lang: Option<&str>,
    dep: Option<&str>,
    severity: Option<&str>,
    layer_filter: LayerFilter,
    detail: Detail,
) -> Value {
    json!({
        "file": file.map(|p| p.display().to_string()),
        "lang": lang,
        "dep": dep,
        "severity": severity,
        "layer_filter": match layer_filter {
            LayerFilter::All => "all",
            LayerFilter::PersonalOnly => "personal",
            LayerFilter::ProjectOnly => "project",
        },
        "detail": match detail {
            Detail::Summary => "summary",
            Detail::Full => "full",
        },
    })
}

// Suppress unused warning for PathBuf reexport in public signatures.
#[allow(dead_code)]
fn _path_marker(_p: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_python_extension() {
        assert_eq!(
            detect_language_from_path(&PathBuf::from("src/app.py")),
            Some("python")
        );
        assert_eq!(
            detect_language_from_path(&PathBuf::from("stubs/app.pyi")),
            Some("python")
        );
    }

    #[test]
    fn detects_typescript_variants() {
        for path in ["a.ts", "a.tsx", "a.js", "a.jsx", "a.mjs", "a.cjs"] {
            assert_eq!(
                detect_language_from_path(&PathBuf::from(path)),
                Some("typescript"),
                "path {path} should be typescript"
            );
        }
    }

    #[test]
    fn detects_rust_extension() {
        assert_eq!(
            detect_language_from_path(&PathBuf::from("src/main.rs")),
            Some("rust")
        );
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert!(detect_language_from_path(&PathBuf::from("README.md")).is_none());
        assert!(detect_language_from_path(&PathBuf::from("Makefile")).is_none());
    }

    #[test]
    fn severity_rank_orders_must_first() {
        assert!(severity_rank("must") < severity_rank("should"));
        assert!(severity_rank("should") < severity_rank("may"));
    }
}
