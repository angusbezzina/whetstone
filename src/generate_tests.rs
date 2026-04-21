//! Generate native test files and linter configurations from approved rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml and renders Tera
//! templates per language:
//! - Python: pytest files + ruff overlay
//! - TypeScript: vitest files + biome overlay
//! - Rust: cargo test files + clippy overlay

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

use crate::rules::{self, ApprovedRule};
use crate::templates::{build_tera, render};

pub fn generate_tests(
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
            "generated": {"tests": [], "lint_configs": []},
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

    let mut test_files: Vec<Value> = Vec::new();
    let mut all_warnings: Vec<String> = warnings;

    for (language, rules) in &by_language {
        match language.as_str() {
            "python" => {
                test_files.extend(generate_python(&tera, rules, &output_base, dry_run));
            }
            "typescript" => {
                test_files.extend(generate_typescript(&tera, rules, &output_base, dry_run));
            }
            "rust" => {
                test_files.extend(generate_rust(&tera, rules, &output_base, dry_run));
            }
            _ => {
                all_warnings.push(format!("Skipping unsupported language: {language}"));
            }
        }
    }

    let eval_root = if personal_output {
        "whetstone/.personal/evals"
    } else {
        "whetstone/evals"
    };

    let next_commands: Vec<String> = by_language
        .keys()
        .filter_map(|lang| match lang.as_str() {
            "python" => Some(format!("python3 -m pytest {eval_root}/python/ -v")),
            "typescript" => Some(format!("npx vitest run {eval_root}/typescript/")),
            "rust" => Some("cargo test --test whetstone_evals".to_string()),
            _ => None,
        })
        .collect();

    Ok(serde_json::json!({
        "status": "ok",
        "generated": {
            "tests": test_files,
        },
        "rules_count": approved.len(),
        "languages": by_language.keys().collect::<Vec<_>>(),
        "warnings": all_warnings,
        "next_command": next_commands.join(" && "),
    }))
}

// ── Per-rule data passed into Tera ──

#[derive(Serialize)]
struct TemplateRule {
    id: String,
    safe_id: String,
    short_desc: String,
    signals: Vec<TemplateSignal>,
}

#[derive(Serialize)]
struct TemplateSignal {
    id: String,
    strategy: String,
    description: String,
    match_pattern: Option<String>,
}

fn to_template_rule(rule: &ApprovedRule) -> TemplateRule {
    let safe_id = rule.id.replace(['.', '-'], "_").replace('@', "");
    let short_desc = rule.description.lines().next().unwrap_or("").to_string();
    let signals = rule
        .signals
        .iter()
        .map(|s| TemplateSignal {
            id: s.id.clone(),
            strategy: s.strategy.clone(),
            description: s.description.clone(),
            match_pattern: s.match_pattern.clone(),
        })
        .collect();
    TemplateRule {
        id: rule.id.clone(),
        safe_id,
        short_desc,
        signals,
    }
}

// ── Python ──

fn generate_python(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut test_files = Vec::new();
    let evals_dir = output_base.join("evals").join("python");

    let conftest = render(tera, "python_conftest.py.tera", &Context::new());
    let conftest_path = evals_dir.join("conftest.py");
    if write_generated(&conftest_path, &conftest, dry_run) {
        test_files.push(serde_json::json!({
            "path": conftest_path.display().to_string(),
            "type": "conftest",
        }));
    }

    let by_dep = group_by_source(rules);

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("test_{safe_name}.py");
        let test_path = evals_dir.join(&test_filename);

        let mut ctx = Context::new();
        ctx.insert("source_name", source_name);
        let tmpl_rules: Vec<TemplateRule> = dep_rules.iter().map(|r| to_template_rule(r)).collect();
        let needs_re = tmpl_rules
            .iter()
            .any(|r| r.signals.iter().any(|s| s.match_pattern.is_some()));
        ctx.insert("needs_re", &needs_re);
        ctx.insert("rules", &tmpl_rules);

        let content = render(tera, "python_test.py.tera", &ctx);
        if write_generated(&test_path, &content, dry_run) {
            for rule in dep_rules {
                test_files.push(serde_json::json!({
                    "path": test_path.display().to_string(),
                    "type": "test",
                    "rule_id": rule.id,
                    "dependency": rule.source_name,
                }));
            }
        }
    }

    test_files
}

// ── TypeScript ──

fn generate_typescript(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut test_files = Vec::new();
    let evals_dir = output_base.join("evals").join("typescript");

    let setup = render(tera, "typescript_setup.ts.tera", &Context::new());
    let setup_path = evals_dir.join("setup.ts");
    if write_generated(&setup_path, &setup, dry_run) {
        test_files.push(serde_json::json!({
            "path": setup_path.display().to_string(),
            "type": "setup",
        }));
    }

    let by_dep = group_by_source(rules);

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("{safe_name}.test.ts");
        let test_path = evals_dir.join(&test_filename);

        let mut ctx = Context::new();
        let tmpl_rules: Vec<TemplateRule> = dep_rules.iter().map(|r| to_template_rule(r)).collect();
        ctx.insert("rules", &tmpl_rules);

        let content = render(tera, "typescript_test.ts.tera", &ctx);
        if write_generated(&test_path, &content, dry_run) {
            for rule in dep_rules {
                test_files.push(serde_json::json!({
                    "path": test_path.display().to_string(),
                    "type": "test",
                    "rule_id": rule.id,
                    "dependency": rule.source_name,
                }));
            }
        }
    }

    test_files
}

// ── Rust ──

fn generate_rust(
    tera: &Tera,
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> Vec<Value> {
    let mut test_files = Vec::new();
    let evals_dir = output_base.join("evals").join("rust");

    let by_dep = group_by_source(rules);

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("test_{safe_name}.rs");
        let test_path = evals_dir.join(&test_filename);

        let mut ctx = Context::new();
        ctx.insert("source_name", source_name);
        let tmpl_rules: Vec<TemplateRule> = dep_rules.iter().map(|r| to_template_rule(r)).collect();
        ctx.insert("rules", &tmpl_rules);

        let content = render(tera, "rust_test.rs.tera", &ctx);
        if write_generated(&test_path, &content, dry_run) {
            for rule in dep_rules {
                test_files.push(serde_json::json!({
                    "path": test_path.display().to_string(),
                    "type": "test",
                    "rule_id": rule.id,
                    "dependency": rule.source_name,
                }));
            }
        }
    }

    test_files
}

// ── Helpers ──

fn group_by_source<'a>(rules: &[&'a ApprovedRule]) -> BTreeMap<String, Vec<&'a ApprovedRule>> {
    let mut by_dep: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in rules {
        by_dep
            .entry(rule.source_name.clone())
            .or_default()
            .push(rule);
    }
    by_dep
}

/// Produce a filesystem-safe identifier from a dependency source name.
fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.chars() {
        let keep = match ch {
            '@' => None,
            'a'..='z' | 'A'..='Z' | '0'..='9' => Some(ch),
            _ => Some('_'),
        };
        if let Some(c) = keep {
            if c == '_' {
                if !last_underscore {
                    out.push('_');
                    last_underscore = true;
                }
            } else {
                out.push(c);
                last_underscore = false;
            }
        }
    }
    out.trim_matches('_').to_string()
}

fn write_generated(path: &Path, content: &str, dry_run: bool) -> bool {
    if dry_run {
        return true;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Tera's per-rule loop trailing whitespace sometimes leaves an extra
    // blank line at EOF; strip it so `ruff format --check` stays green on
    // regenerated fixtures without anyone running `ruff format` by hand.
    let normalized = normalize_trailing_newline(content);
    std::fs::write(path, normalized).is_ok()
}

fn normalize_trailing_newline(content: &str) -> String {
    let trimmed = content.trim_end_matches(|c: char| c == '\n' || c == '\r');
    let mut out = String::with_capacity(trimmed.len() + 1);
    out.push_str(trimmed);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved(id: &str, strategy: &str, match_pattern: Option<&str>) -> ApprovedRule {
        ApprovedRule {
            id: id.to_string(),
            severity: "must".into(),
            confidence: "high".into(),
            category: "default".into(),
            description: "desc".into(),
            source_url: "https://example".into(),
            source_name: "demo".into(),
            language: "python".into(),
            signals: vec![rules::ApprovedSignal {
                id: "s1".into(),
                strategy: strategy.into(),
                description: "signal".into(),
                weight: "required".into(),
                match_pattern: match_pattern.map(String::from),
                ast_query: None,
                ast_scope: None,
            }],
            golden_examples: Vec::new(),
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
        }
    }

    #[test]
    fn python_template_embeds_match_regex() {
        let tera = build_tera();
        let rule = approved("demo.foo", "pattern", Some(r"\.unwrap\(\)"));
        let mut ctx = Context::new();
        ctx.insert("source_name", "demo");
        let tmpl_rules = vec![to_template_rule(&rule)];
        ctx.insert("rules", &tmpl_rules);
        let out = render(&tera, "python_test.py.tera", &ctx);
        assert!(out.contains("def test_demo_foo_signal_0"), "got: {out}");
        assert!(
            out.contains(r#"re.search(r"""\.unwrap\(\)""", line)"#),
            "got: {out}"
        );
    }

    #[test]
    fn python_template_emits_todo_stub_when_no_match() {
        let tera = build_tera();
        let rule = approved("demo.bar", "pattern", None);
        let mut ctx = Context::new();
        ctx.insert("source_name", "demo");
        let tmpl_rules = vec![to_template_rule(&rule)];
        ctx.insert("rules", &tmpl_rules);
        let out = render(&tera, "python_test.py.tera", &ctx);
        assert!(out.contains("# TODO: add `match:` regex to rule demo.bar signal s1"));
    }

    #[test]
    fn sanitize_name_strips_at_and_folds_separators() {
        assert_eq!(sanitize_name("@scope/pkg-name.v1"), "scope_pkg_name_v1");
        assert_eq!(
            sanitize_name("whetstone:recommended/python"),
            "whetstone_recommended_python"
        );
    }
}
