//! Generate native test files and linter configurations from approved rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml and generates:
//! - Python: pytest files + ruff config overlay
//! - TypeScript: vitest files + biome config overlay
//! - Rust: cargo test files + clippy config overlay

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

use crate::rules::{self, ApprovedRule};

pub fn generate_tests(
    project_dir: &Path,
    lang_filter: Option<&str>,
    dry_run: bool,
    personal_output: bool,
) -> Result<Value> {
    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let (approved, warnings, output_base): (Vec<ApprovedRule>, Vec<String>, std::path::PathBuf) =
        if personal_output {
            let (rules, warns) = crate::layers::load_personal_only(project_dir, lang_filter);
            (rules, warns, paths.personal_dir.clone())
        } else if whetstone_config_exists {
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
            "warnings": ["No approved rules found. Run 'wh doctor' to extract and approve rules."],
            "next_command": "wh doctor",
        }));
    }

    let mut by_language: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in &approved {
        by_language
            .entry(rule.language.clone())
            .or_default()
            .push(rule);
    }

    let mut test_files: Vec<Value> = Vec::new();
    let mut lint_configs: Vec<Value> = Vec::new();
    let mut all_warnings: Vec<String> = warnings;

    for (language, rules) in &by_language {
        match language.as_str() {
            "python" => {
                let (tests, lints, warns) = generate_python(rules, &output_base, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
            }
            "typescript" => {
                let (tests, lints, warns) = generate_typescript(rules, &output_base, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
            }
            "rust" => {
                let (tests, lints, warns) = generate_rust(rules, &output_base, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
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
            "lint_configs": lint_configs,
        },
        "rules_count": approved.len(),
        "languages": by_language.keys().collect::<Vec<_>>(),
        "warnings": all_warnings,
        "next_command": next_commands.join(" && "),
    }))
}

// --- Python test generation ---

fn generate_python(
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let evals_dir = output_base.join("evals").join("python");

    // Generate conftest.py
    let conftest = generate_python_conftest();
    let conftest_path = evals_dir.join("conftest.py");
    if write_generated(&conftest_path, &conftest, dry_run) {
        test_files.push(serde_json::json!({
            "path": conftest_path.display().to_string(),
            "type": "conftest",
        }));
    }

    // Group rules by source_name so a rule file with N rules writes a single
    // test file containing N tests instead of N files that overwrite each other.
    let mut by_dep: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in rules {
        by_dep
            .entry(rule.source_name.clone())
            .or_default()
            .push(rule);
    }

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("test_{safe_name}.py");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_python_test_file(source_name, dep_rules);
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

    // Generate ruff overlay config
    let ruff_rules = extract_lint_proxy_codes(rules, "ruff");
    if !ruff_rules.is_empty() {
        let ruff_content = generate_ruff_config(&ruff_rules);
        let ruff_path = output_base.join("lint").join("ruff.whetstone.toml");
        if write_generated(&ruff_path, &ruff_content, dry_run) {
            lint_configs.push(serde_json::json!({
                "path": ruff_path.display().to_string(),
                "type": "ruff",
                "rules": ruff_rules,
            }));
        }
    }

    (test_files, lint_configs, Vec::new())
}

fn generate_python_conftest() -> String {
    r#""""Shared fixtures for Whetstone Python eval tests."""
import glob
import os

import pytest


@pytest.fixture
def python_source_files():
    """Find all Python source files in the project (excluding tests/venv)."""
    skip = {"venv", ".venv", "node_modules", "__pycache__", ".git", "whetstone"}
    files = []
    for root, dirs, filenames in os.walk("src"):
        dirs[:] = [d for d in dirs if d not in skip]
        for f in filenames:
            if f.endswith(".py"):
                files.append(os.path.join(root, f))
    if not files:
        for root, dirs, filenames in os.walk("."):
            dirs[:] = [d for d in dirs if d not in skip]
            for f in filenames:
                if f.endswith(".py"):
                    files.append(os.path.join(root, f))
    return files
"#
    .to_string()
}

fn generate_python_test_file(source_name: &str, rules: &[&ApprovedRule]) -> String {
    let mut out = Vec::new();
    out.push(format!(
        "\"\"\"Whetstone evals for dependency: {source_name}.\"\"\""
    ));
    out.push(String::new());
    out.push("import re".to_string());
    out.push(String::new());
    out.push(String::new());
    for rule in rules {
        out.push(generate_python_test(rule));
    }
    out.join("\n")
}

fn generate_python_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let safe_id = rule.id.replace(['.', '-'], "_");

    lines.push(format!(
        "# Rule: {} — {}",
        rule.id,
        rule.description.lines().next().unwrap_or("")
    ));

    for (i, signal) in rule.signals.iter().enumerate() {
        let test_name = format!("test_{safe_id}_signal_{i}");
        match signal.strategy.as_str() {
            "pattern" | "ast" => {
                let ast_hint = if signal.strategy == "ast" {
                    // Until tree-sitter lands, `ast` signals still scan as regex.
                    // Flag this in-source so readers know the check is weaker than
                    // a true AST walk.
                    "    # NOTE: ast signal regex fallback — replace with tree-sitter when available.\n"
                } else {
                    ""
                };
                lines.push(format!("def {test_name}(python_source_files):"));
                lines.push(format!(
                    "    \"\"\"Signal: {} ({})\"\"\"",
                    signal.description, signal.strategy
                ));
                lines.push("    violations = []".to_string());
                lines.push("    for filepath in python_source_files:".to_string());
                lines.push("        with open(filepath, encoding=\"utf-8\") as f:".to_string());
                lines.push("            for lineno, line in enumerate(f, 1):".to_string());

                if let Some(ref pattern) = signal.match_pattern {
                    // Real regex check from the rule's match field.
                    // Emit as a Python raw triple-quoted string so embedded quotes and backslashes survive.
                    lines.push(format!(
                        "                if re.search(r\"\"\"{}\"\"\", line):",
                        pattern.replace("\"\"\"", "\\\"\\\"\\\"")
                    ));
                    lines.push(
                        "                    violations.append(f\"{filepath}:{lineno}: {line.strip()}\")"
                            .to_string(),
                    );
                } else {
                    // No concrete match regex — emit a TODO stub that documents the rule
                    // but enforces nothing. Extraction should add a `match:` field to
                    // upgrade this from a stub to a real check.
                    lines.push(format!(
                        "                pass  # TODO: add `match:` regex to rule {} signal {} to enable this check.",
                        rule.id, signal.id
                    ));
                }

                if !ast_hint.is_empty() {
                    lines.push(ast_hint.trim_end().to_string());
                }
                lines.push(format!(
                    "    assert not violations, f\"{{len(violations)}} violation(s) for {}: {{violations[:5]}}\"",
                    rule.id
                ));
                lines.push(String::new());
            }
            "lint_proxy" => {
                lines.push(format!(
                    "# Signal {i}: {} — deferred to ruff linter config",
                    signal.description
                ));
                lines.push(String::new());
            }
            "ai" => {
                lines.push(format!(
                    "# Signal {i}: {} — deferred to `wh eval run`",
                    signal.description
                ));
                lines.push(String::new());
            }
            _ => {
                lines.push(format!(
                    "# Signal {i}: {} — {} (not auto-testable)",
                    signal.description, signal.strategy
                ));
                lines.push(String::new());
            }
        }
    }

    lines.join("\n")
}

// --- TypeScript test generation ---

fn generate_typescript(
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let warnings = Vec::new();

    let evals_dir = output_base.join("evals").join("typescript");

    // Generate setup.ts
    let setup = generate_ts_setup();
    let setup_path = evals_dir.join("setup.ts");
    if write_generated(&setup_path, &setup, dry_run) {
        test_files.push(serde_json::json!({
            "path": setup_path.display().to_string(),
            "type": "setup",
        }));
    }

    // Group rules by source_name so a dep with N rules writes a single file
    // with N describe blocks instead of N files that overwrite each other.
    let mut by_dep: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in rules {
        by_dep
            .entry(rule.source_name.clone())
            .or_default()
            .push(rule);
    }

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("{safe_name}.test.ts");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_ts_test_file(dep_rules);
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

    // Generate biome overlay config
    let biome_rules = extract_lint_proxy_codes(rules, "biome");
    if !biome_rules.is_empty() {
        let biome_content = generate_biome_config(&biome_rules);
        let biome_path = output_base.join("lint").join("biome.whetstone.json");
        if write_generated(&biome_path, &biome_content, dry_run) {
            lint_configs.push(serde_json::json!({
                "path": biome_path.display().to_string(),
                "type": "biome",
                "rules": biome_rules,
            }));
        }
    }

    (test_files, lint_configs, warnings)
}

fn generate_ts_setup() -> String {
    r#"import { glob } from 'glob';
import { readFileSync } from 'fs';

export function findSourceFiles(patterns: string[] = ['src/**/*.ts', 'src/**/*.tsx']): string[] {
  return patterns.flatMap(p => glob.sync(p, { ignore: ['**/node_modules/**', '**/whetstone/**'] }));
}

export function readLines(filepath: string): string[] {
  return readFileSync(filepath, 'utf-8').split('\n');
}

export interface Violation {
  file: string;
  line: number;
  text: string;
}

export function violation(file: string, line: number, text: string): Violation {
  return { file, line, text };
}
"#
    .to_string()
}

fn generate_ts_test_file(rules: &[&ApprovedRule]) -> String {
    let mut out = Vec::new();
    out.push("import { describe, it, expect } from 'vitest';".to_string());
    out.push("import { findSourceFiles, readLines } from './setup';".to_string());
    out.push(String::new());
    for rule in rules {
        out.push(generate_ts_test(rule));
        out.push(String::new());
    }
    out.join("\n")
}

fn generate_ts_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let _safe_id = rule.id.replace(['.', '-'], "_").replace('@', "");

    lines.push(format!(
        "describe('{}', () => {{",
        rule.id.replace('\'', "\\'")
    ));

    for (i, signal) in rule.signals.iter().enumerate() {
        match signal.strategy.as_str() {
            "pattern" | "ast" => {
                lines.push(format!(
                    "  it('signal {i}: {}', () => {{",
                    signal.description.replace('\'', "\\'")
                ));
                lines.push("    const files = findSourceFiles();".to_string());
                lines.push("    const violations: string[] = [];".to_string());

                if let Some(ref pattern) = signal.match_pattern {
                    // Real regex check using the rule's match field. Escape backticks
                    // so the regex can live inside a TypeScript template literal safely.
                    let escaped = pattern.replace('\\', "\\\\").replace('`', "\\`");
                    lines.push(format!("    const pattern = new RegExp(`{escaped}`);"));
                    lines.push("    for (const file of files) {".to_string());
                    lines.push("      const lines = readLines(file);".to_string());
                    lines.push("      lines.forEach((line, idx) => {".to_string());
                    lines.push("        if (pattern.test(line)) {".to_string());
                    lines.push(
                        "          violations.push(`${file}:${idx + 1}: ${line.trim()}`);"
                            .to_string(),
                    );
                    lines.push("        }".to_string());
                    lines.push("      });".to_string());
                    lines.push("    }".to_string());
                } else {
                    // No concrete match regex — TODO stub that documents the gap.
                    lines.push("    for (const file of files) {".to_string());
                    lines.push("      const lines = readLines(file);".to_string());
                    lines.push("      lines.forEach((line, idx) => {".to_string());
                    lines.push(format!(
                        "        // TODO: add `match:` regex to rule {} signal {} to enable this check.",
                        rule.id, signal.id
                    ));
                    lines.push("      });".to_string());
                    lines.push("    }".to_string());
                }

                if signal.strategy == "ast" {
                    lines.push(
                        "    // NOTE: ast signal regex fallback — upgrade to tree-sitter when available."
                            .to_string(),
                    );
                }
                lines.push(format!(
                    "    expect(violations).toEqual([]);  // {}",
                    rule.id
                ));
                lines.push("  });".to_string());
            }
            "lint_proxy" => {
                lines.push(format!(
                    "  // signal {i}: {} — deferred to biome config",
                    signal.description
                ));
            }
            "ai" => {
                lines.push(format!(
                    "  // signal {i}: {} — deferred to `wh eval run`",
                    signal.description
                ));
            }
            _ => {
                lines.push(format!(
                    "  // signal {i}: {} — {} (not auto-testable)",
                    signal.description, signal.strategy
                ));
            }
        }
        lines.push(String::new());
    }

    lines.push("});".to_string());
    lines.join("\n")
}

// --- Rust test generation ---

fn generate_rust(
    rules: &[&ApprovedRule],
    output_base: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let warnings = Vec::new();

    let evals_dir = output_base.join("evals").join("rust");

    // Group rules by source_name so deps with multiple rules share one test file.
    let mut by_dep: BTreeMap<String, Vec<&ApprovedRule>> = BTreeMap::new();
    for rule in rules {
        by_dep
            .entry(rule.source_name.clone())
            .or_default()
            .push(rule);
    }

    for (source_name, dep_rules) in &by_dep {
        let safe_name = sanitize_name(source_name);
        let test_filename = format!("test_{safe_name}.rs");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_rust_test_file(source_name, dep_rules);
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

    // Generate clippy overlay
    let clippy_rules = extract_lint_proxy_codes(rules, "clippy");
    if !clippy_rules.is_empty() {
        let clippy_content = generate_clippy_config(&clippy_rules);
        let clippy_path = output_base.join("lint").join("clippy.whetstone.toml");
        if write_generated(&clippy_path, &clippy_content, dry_run) {
            lint_configs.push(serde_json::json!({
                "path": clippy_path.display().to_string(),
                "type": "clippy",
                "rules": clippy_rules,
            }));
        }
    }

    (test_files, lint_configs, warnings)
}

fn generate_rust_test_file(source_name: &str, rules: &[&ApprovedRule]) -> String {
    let mut out = Vec::new();
    out.push(format!("//! Whetstone evals for dependency: {source_name}"));
    out.push(String::new());
    out.push("use std::fs;".to_string());
    out.push("use std::path::Path;".to_string());
    out.push(String::new());
    out.push(FIND_RUST_FILES_HELPER.to_string());
    out.push(String::new());
    for rule in rules {
        out.push(generate_rust_test(rule));
        out.push(String::new());
    }
    out.join("\n")
}

const FIND_RUST_FILES_HELPER: &str = "fn find_rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !matches!(name.as_ref(), \"target\" | \".git\" | \"whetstone\") {
                    files.extend(find_rust_files(&path));
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some(\"rs\") {
                files.push(path);
            }
        }
    }
    files
}";

fn generate_rust_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let safe_id = rule.id.replace(['.', '-'], "_");

    lines.push(format!(
        "// Rule: {} — {}",
        rule.id,
        rule.description.lines().next().unwrap_or("")
    ));

    for (i, signal) in rule.signals.iter().enumerate() {
        let test_name = format!("test_{safe_id}_signal_{i}");
        match signal.strategy.as_str() {
            "pattern" | "ast" => {
                lines.push("#[test]".to_string());
                lines.push(format!("fn {test_name}() {{"));
                lines.push(format!(
                    "    // Signal: {} ({})",
                    signal.description, signal.strategy
                ));
                lines.push("    let files = find_rust_files(Path::new(\"src\"));".to_string());
                lines.push("    let mut violations = Vec::new();".to_string());

                if signal.strategy == "ast" {
                    lines.push(
                        "    // NOTE: ast signal regex fallback — upgrade to tree-sitter when available."
                            .to_string(),
                    );
                }
                if let Some(ref pattern) = signal.match_pattern {
                    // Real regex check using the match pattern
                    // Pattern goes inside r"..." raw string — no extra escaping needed
                    // except for double quotes which would close the raw string
                    let escaped = pattern.replace('"', "'");
                    lines.push(format!(
                        "    let pattern = regex::Regex::new(r\"{escaped}\").unwrap();"
                    ));
                    lines.push("    for file in &files {".to_string());
                    lines.push(
                        "        if let Ok(content) = fs::read_to_string(file) {".to_string(),
                    );
                    lines.push(
                        "            for (line_num, line) in content.lines().enumerate() {"
                            .to_string(),
                    );
                    lines.push("                if pattern.is_match(line) {".to_string());
                    lines.push(
                        "                    violations.push(format!(\"{}:{}: {}\", file.display(), line_num + 1, line.trim()));".to_string(),
                    );
                    lines.push("                }".to_string());
                    lines.push("            }".to_string());
                    lines.push("        }".to_string());
                    lines.push("    }".to_string());
                } else {
                    // No `match:` regex on the signal — produce a TODO stub so the
                    // test compiles but enforces nothing. Extraction should add a
                    // concrete `match:` regex to upgrade this to a real check.
                    lines.push("    for file in &files {".to_string());
                    lines.push(
                        "        if let Ok(content) = fs::read_to_string(file) {".to_string(),
                    );
                    lines.push(format!(
                        "            // TODO: add `match:` regex to rule {} signal {} to enable this check.",
                        rule.id, signal.id
                    ));
                    lines.push("            let _ = content;".to_string());
                    lines.push("        }".to_string());
                    lines.push("    }".to_string());
                }

                lines.push(format!(
                    "    assert!(violations.is_empty(), \"{{}} violations for {}:\\n{{}}\", violations.len(), violations.join(\"\\n\"));",
                    rule.id
                ));
                lines.push("}".to_string());
                lines.push(String::new());
            }
            "lint_proxy" => {
                lines.push(format!(
                    "// Signal {i}: {} — deferred to clippy config",
                    signal.description
                ));
                lines.push(String::new());
            }
            "ai" => {
                lines.push(format!(
                    "// Signal {i}: {} — deferred to `wh eval run`",
                    signal.description
                ));
                lines.push(String::new());
            }
            _ => {
                lines.push(format!(
                    "// Signal {i}: {} — {} (not auto-testable)",
                    signal.description, signal.strategy
                ));
                lines.push(String::new());
            }
        }
    }

    lines.join("\n")
}

// --- Linter config generation ---

fn extract_lint_proxy_codes(rules: &[&ApprovedRule], linter: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for rule in rules {
        for signal in &rule.signals {
            if signal.strategy == "lint_proxy" {
                let desc = &signal.description;
                // Try to extract lint rule codes from the description
                // e.g., "ruff E501" or "biome suspicious/noExplicitAny"
                if desc.to_lowercase().contains(linter) {
                    // Extract the code part after the linter name
                    let parts: Vec<&str> = desc.split_whitespace().collect();
                    for (i, part) in parts.iter().enumerate() {
                        if part.to_lowercase() == linter && i + 1 < parts.len() {
                            codes.push(parts[i + 1].to_string());
                        }
                    }
                }
            }
        }
    }
    codes.sort();
    codes.dedup();
    codes
}

fn generate_ruff_config(codes: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push("# Whetstone ruff overlay — extend your ruff.toml with these rules".to_string());
    lines.push("[lint]".to_string());
    lines.push(format!(
        "select = [{}]",
        codes
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.join("\n")
}

fn generate_biome_config(rules: &[String]) -> String {
    let mut obj = serde_json::json!({
        "$schema": "https://biomejs.dev/schemas/1.9.0/schema.json",
        "linter": {
            "rules": {}
        }
    });

    for rule in rules {
        let parts: Vec<&str> = rule.split('/').collect();
        if parts.len() == 2 {
            obj["linter"]["rules"][parts[0]][parts[1]] = serde_json::json!("error");
        }
    }

    serde_json::to_string_pretty(&obj).unwrap_or_default()
}

fn generate_clippy_config(lints: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push("# Whetstone clippy overlay".to_string());
    for lint in lints {
        lines.push(format!("warn = [\"{lint}\"]"));
    }
    lines.join("\n")
}

// --- Helpers ---

/// Produce a filesystem-safe identifier from a dependency source name.
/// Strips `@`, folds `/`, `:`, `-`, `.`, and whitespace to `_`, and collapses
/// runs so `whetstone:recommended/python` → `whetstone_recommended_python`.
fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.chars() {
        let keep = match ch {
            '@' => None, // drop entirely
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
    // Trim leading/trailing underscores so filenames do not start/end with `_`.
    out.trim_matches('_').to_string()
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
