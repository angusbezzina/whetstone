//! Generate linter and formatter configuration overlays from approved rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml, mines `lint_proxy`
//! signals plus any optional formatter directives, and writes one config per
//! supported enforcement surface:
//! - Python: `whetstone/lint/ruff.whetstone.toml`
//! - TypeScript: `whetstone/lint/biome.whetstone.json`
//! - Rust lint: `whetstone/lint/clippy.whetstone.toml`
//! - Rust formatting: `whetstone/lint/rustfmt.whetstone.toml`

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

use crate::rules::{self, ApprovedRule};
use crate::templates::{build_tera, render};

const RUFF_FORMAT_KEYS: &[&str] = &[
    "quote-style",
    "indent-style",
    "line-ending",
    "docstring-code-format",
    "docstring-code-line-length",
];
const BIOME_FORMAT_KEYS: &[&str] = &[
    "quoteStyle",
    "jsxQuoteStyle",
    "quoteProperties",
    "trailingCommas",
    "semicolons",
    "arrowParentheses",
    "bracketSpacing",
    "bracketSameLine",
    "lineWidth",
    "indentStyle",
    "indentWidth",
];
const RUSTFMT_KEYS: &[&str] = &[
    "max_width",
    "hard_tabs",
    "newline_style",
    "use_small_heuristics",
    "tab_spaces",
];

#[derive(Clone, serde::Serialize)]
struct RenderedOption {
    key: String,
    value: String,
}

pub fn generate_lint(
    project_dir: &Path,
    lang_filter: Option<&str>,
    dry_run: bool,
    personal_output: bool,
) -> Result<Value> {
    let project_initialized = crate::layers::project_is_initialized(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let (approved, warnings, output_base): (Vec<ApprovedRule>, Vec<String>, PathBuf) =
        if personal_output {
            let (rules, warns) = crate::layers::load_personal_only(project_dir, lang_filter);
            (rules, warns, paths.personal_dir.clone())
        } else if project_initialized {
            let merged =
                crate::layers::resolve_merged(project_dir, lang_filter, true, false, false);
            let approved = merged.merged.into_iter().map(|lr| lr.rule).collect();
            (approved, merged.warnings, paths.whetstone_dir.clone())
        } else {
            let (approved, warns) =
                rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
            (approved, warns, paths.whetstone_dir.clone())
        };

    if approved.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": {"lint_configs": [], "formatter_configs": []},
            "warnings": ["No approved rules found. Run 'wh init' to extract and approve rules."],
            "next_command": "wh init",
        }));
    }

    let tera = build_tera();

    let mut by_language: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in &approved {
        by_language
            .entry(rule.language.clone())
            .or_default()
            .push(rule);
    }

    let mut lint_configs: Vec<Value> = Vec::new();
    let mut formatter_configs: Vec<Value> = Vec::new();
    let mut all_warnings: Vec<String> = warnings;

    for (language, rules) in &by_language {
        match language.as_str() {
            "python" => {
                let (lints, formatters, mut warns) =
                    generate_python_lint(&tera, rules, &output_base, dry_run);
                lint_configs.extend(lints);
                formatter_configs.extend(formatters);
                all_warnings.append(&mut warns);
            }
            "typescript" => {
                let (lints, formatters, mut warns) =
                    generate_typescript_lint(&tera, rules, &output_base, dry_run);
                lint_configs.extend(lints);
                formatter_configs.extend(formatters);
                all_warnings.append(&mut warns);
            }
            "rust" => {
                let (lints, formatters, mut warns) =
                    generate_rust_lint(&tera, rules, &output_base, dry_run);
                lint_configs.extend(lints);
                formatter_configs.extend(formatters);
                all_warnings.append(&mut warns);
            }
            _ => all_warnings.push(format!("Skipping unsupported language: {language}")),
        }
    }

    Ok(serde_json::json!({
        "status": "ok",
        "generated": {
            "lint_configs": lint_configs,
            "formatter_configs": formatter_configs,
        },
        "rules_count": approved.len(),
        "languages": by_language.keys().collect::<Vec<_>>(),
        "warnings": all_warnings,
    }))
}

// ── Per-language emitters ──

fn generate_python_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut lint_out = Vec::new();
    let mut formatter_out = Vec::new();
    let mut warnings = Vec::new();

    let ruff_rules = extract_lint_proxy_codes(rules, "ruff");
    let (format_options, mut format_warnings) =
        extract_formatter_options(rules, "ruff", RUFF_FORMAT_KEYS);
    warnings.append(&mut format_warnings);

    if ruff_rules.is_empty() && format_options.is_empty() {
        return (lint_out, formatter_out, warnings);
    }

    let mut ctx = Context::new();
    ctx.insert("codes", &ruff_rules);
    ctx.insert("format_options", &rendered_toml_options(&format_options));
    let content = render(tera, "ruff_config.tera", &ctx);
    let path = output_base.join("lint").join("ruff.whetstone.toml");

    if write_generated(&path, &content, dry_run) {
        if !ruff_rules.is_empty() {
            lint_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "ruff",
                "rules": ruff_rules,
            }));
        }
        if !format_options.is_empty() {
            formatter_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "ruff_format",
                "options": format_options,
            }));
        }
    }

    (lint_out, formatter_out, warnings)
}

fn generate_typescript_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut lint_out = Vec::new();
    let mut formatter_out = Vec::new();
    let mut warnings = Vec::new();

    let biome_rules = extract_lint_proxy_codes(rules, "biome");
    let (format_options, mut format_warnings) =
        extract_formatter_options(rules, "biome", BIOME_FORMAT_KEYS);
    warnings.append(&mut format_warnings);

    if biome_rules.is_empty() && format_options.is_empty() {
        return (lint_out, formatter_out, warnings);
    }

    let grouped = group_biome_rules(&biome_rules);
    let mut ctx = Context::new();
    ctx.insert("groups", &grouped);
    ctx.insert("format_options", &rendered_json_options(&format_options));
    let content = render(tera, "biome_config.tera", &ctx);
    let path = output_base.join("lint").join("biome.whetstone.json");

    if write_generated(&path, &content, dry_run) {
        if !biome_rules.is_empty() {
            lint_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "biome",
                "rules": biome_rules,
            }));
        }
        if !format_options.is_empty() {
            formatter_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "biome_formatter",
                "options": format_options,
            }));
        }
    }

    (lint_out, formatter_out, warnings)
}

fn generate_rust_lint(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut lint_out = Vec::new();
    let mut formatter_out = Vec::new();
    let mut warnings = Vec::new();

    let clippy_rules = extract_lint_proxy_codes(rules, "clippy");
    let (format_options, mut format_warnings) =
        extract_formatter_options(rules, "rustfmt", RUSTFMT_KEYS);
    warnings.append(&mut format_warnings);

    if !clippy_rules.is_empty() {
        let mut ctx = Context::new();
        ctx.insert("lints", &clippy_rules);
        let content = render(tera, "clippy_config.tera", &ctx);
        let path = output_base.join("lint").join("clippy.whetstone.toml");
        if write_generated(&path, &content, dry_run) {
            lint_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "clippy",
                "rules": clippy_rules,
            }));
        }
    }

    if !format_options.is_empty() {
        let mut ctx = Context::new();
        ctx.insert("options", &rendered_toml_options(&format_options));
        let content = render(tera, "rustfmt_config.tera", &ctx);
        let path = output_base.join("lint").join("rustfmt.whetstone.toml");
        if write_generated(&path, &content, dry_run) {
            formatter_out.push(serde_json::json!({
                "path": path.display().to_string(),
                "type": "rustfmt",
                "options": format_options,
            }));
        }
    }

    (lint_out, formatter_out, warnings)
}

// ── Helpers ──

fn extract_lint_proxy_codes(rules: &[&ApprovedRule], linter: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for rule in rules {
        for signal in &rule.signals {
            if signal.strategy != "lint_proxy" {
                continue;
            }
            for binding in rules::approved_signal_lint_bindings(signal) {
                if binding.tool == linter {
                    codes.push(binding.code);
                }
            }
        }
    }
    codes.sort();
    codes.dedup();
    codes
}

fn extract_formatter_options(
    rules: &[&ApprovedRule],
    tool: &str,
    allowed_keys: &[&str],
) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let mut warnings = Vec::new();

    for rule in rules {
        let Some(formatter) = &rule.formatter else {
            continue;
        };
        if formatter.tool != tool {
            continue;
        }
        for (key, value) in &formatter.options {
            if !allowed_keys.contains(&key.as_str()) {
                warnings.push(format!(
                    "Skipping formatter option `{key}` from rule `{}` for tool `{tool}`; allowed keys: {}",
                    rule.id,
                    allowed_keys.join(", ")
                ));
                continue;
            }
            if !matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                warnings.push(format!(
                    "Skipping formatter option `{key}` from rule `{}` for tool `{tool}`; only string, number, and boolean values are supported",
                    rule.id
                ));
                continue;
            }
            if let Some(existing) = out.get(key) {
                if existing != value {
                    warnings.push(format!(
                        "Formatter option conflict for `{key}` on tool `{tool}`; later rule `{}` overrides the previous value",
                        rule.id
                    ));
                }
            }
            out.insert(key.clone(), value.clone());
        }
    }

    (out, warnings)
}

fn rendered_toml_options(options: &BTreeMap<String, Value>) -> Vec<RenderedOption> {
    options
        .iter()
        .filter_map(|(key, value)| {
            toml_scalar(value).map(|rendered| RenderedOption {
                key: key.clone(),
                value: rendered,
            })
        })
        .collect()
}

fn rendered_json_options(options: &BTreeMap<String, Value>) -> Vec<RenderedOption> {
    options
        .iter()
        .map(|(key, value)| RenderedOption {
            key: key.clone(),
            value: serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
        })
        .collect()
}

fn toml_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(format!("\"{}\"", s.replace('"', "\\\""))),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn group_biome_rules(rules: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rule in rules {
        if let Some((category, name)) = rule.split_once('/') {
            out.entry(category.to_string())
                .or_default()
                .push(name.to_string());
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

fn write_generated(path: &Path, content: &str, dry_run: bool) -> bool {
    if dry_run {
        return true;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, content).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved_with_formatter(tool: &str, options: &[(&str, Value)]) -> ApprovedRule {
        ApprovedRule {
            id: format!("demo.{tool}"),
            severity: "must".into(),
            confidence: "high".into(),
            category: "convention".into(),
            description: "desc".into(),
            source_url: "https://example".into(),
            source_name: "demo".into(),
            language: match tool {
                "biome" => "typescript".into(),
                "rustfmt" => "rust".into(),
                _ => "python".into(),
            },
            signals: Vec::new(),
            formatter: Some(rules::ApprovedFormatterDirective {
                tool: tool.into(),
                options: options
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            }),
            tests: Vec::new(),
            golden_examples: Vec::new(),
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
        }
    }

    #[test]
    fn structured_lint_binding_is_preferred() {
        let rule = ApprovedRule {
            id: "demo.b006".into(),
            severity: "must".into(),
            confidence: "high".into(),
            category: "default".into(),
            description: "desc".into(),
            source_url: "https://example".into(),
            source_name: "demo".into(),
            language: "python".into(),
            signals: vec![rules::ApprovedSignal {
                id: "lint".into(),
                strategy: "lint_proxy".into(),
                description: "legacy text".into(),
                weight: "required".into(),
                match_pattern: None,
                ast_query: None,
                ast_scope: None,
                lint: Some(rules::ApprovedLintBinding {
                    tool: "ruff".into(),
                    code: "B006".into(),
                }),
            }],
            formatter: None,
            tests: Vec::new(),
            golden_examples: Vec::new(),
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
        };

        assert_eq!(extract_lint_proxy_codes(&[&rule], "ruff"), vec!["B006"]);
    }

    #[test]
    fn formatter_conflicts_warn_and_last_value_wins() {
        let first =
            approved_with_formatter("ruff", &[("quote-style", Value::String("single".into()))]);
        let second =
            approved_with_formatter("ruff", &[("quote-style", Value::String("double".into()))]);
        let (options, warnings) =
            extract_formatter_options(&[&first, &second], "ruff", RUFF_FORMAT_KEYS);
        assert_eq!(
            options.get("quote-style").and_then(|v| v.as_str()),
            Some("double")
        );
        assert!(warnings.iter().any(|w| w.contains("conflict")));
    }

    #[test]
    fn rendered_toml_options_quote_strings() {
        let rendered = rendered_toml_options(&BTreeMap::from([(
            "quote-style".to_string(),
            Value::String("single".into()),
        )]));
        assert_eq!(rendered[0].value, "\"single\"");
    }
}
