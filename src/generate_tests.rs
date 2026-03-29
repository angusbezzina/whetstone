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
) -> Result<Value> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (approved, warnings) = rules::load_approved_rules(&rules_dir, lang_filter);

    if approved.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": {"tests": [], "lint_configs": []},
            "warnings": ["No approved rules found. Run 'whetstone doctor' to extract and approve rules."],
            "next_command": "whetstone doctor",
        }));
    }

    // Group rules by language
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
                let (tests, lints, warns) =
                    generate_python(&rules, project_dir, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
            }
            "typescript" => {
                let (tests, lints, warns) =
                    generate_typescript(&rules, project_dir, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
            }
            "rust" => {
                let (tests, lints, warns) =
                    generate_rust(&rules, project_dir, dry_run);
                test_files.extend(tests);
                lint_configs.extend(lints);
                all_warnings.extend(warns);
            }
            _ => {
                all_warnings.push(format!("Skipping unsupported language: {language}"));
            }
        }
    }

    let next_commands: Vec<String> = by_language
        .keys()
        .filter_map(|lang| match lang.as_str() {
            "python" => Some("python3 -m pytest whetstone/evals/python/ -v".to_string()),
            "typescript" => Some("npx vitest run whetstone/evals/typescript/".to_string()),
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
    project_dir: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let evals_dir = project_dir.join("whetstone").join("evals").join("python");

    // Generate conftest.py
    let conftest = generate_python_conftest();
    let conftest_path = evals_dir.join("conftest.py");
    if write_generated(&conftest_path, &conftest, dry_run) {
        test_files.push(serde_json::json!({
            "path": conftest_path.display().to_string(),
            "type": "conftest",
        }));
    }

    // Generate test file per dependency
    for rule in rules {
        let safe_name = rule.source_name.replace('-', "_").replace('.', "_");
        let test_filename = format!("test_{safe_name}.py");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_python_test(rule);
        if write_generated(&test_path, &content, dry_run) {
            test_files.push(serde_json::json!({
                "path": test_path.display().to_string(),
                "type": "test",
                "rule_id": rule.id,
                "dependency": rule.source_name,
            }));
        }
    }

    // Generate ruff overlay config
    let ruff_rules = extract_lint_proxy_codes(rules, "ruff");
    if !ruff_rules.is_empty() {
        let ruff_content = generate_ruff_config(&ruff_rules);
        let ruff_path = project_dir
            .join("whetstone")
            .join("lint")
            .join("ruff.whetstone.toml");
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

fn generate_python_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let safe_id = rule.id.replace('.', "_").replace('-', "_");

    lines.push(format!(
        "\"\"\"Whetstone eval: {} — {}\"\"\"\n",
        rule.id,
        rule.description.lines().next().unwrap_or("")
    ));
    lines.push("import ast".to_string());
    lines.push("import re".to_string());
    lines.push("import os".to_string());
    lines.push(String::new());
    lines.push(String::new());

    for (i, signal) in rule.signals.iter().enumerate() {
        let test_name = format!("test_{safe_id}_signal_{i}");
        match signal.strategy.as_str() {
            "pattern" => {
                lines.push(format!("def {test_name}(python_source_files):"));
                lines.push(format!(
                    "    \"\"\"Signal: {} (pattern)\"\"\"\n    violations = []",
                    signal.description
                ));
                lines.push("    for filepath in python_source_files:".to_string());
                lines.push("        with open(filepath) as f:".to_string());
                lines.push("            for lineno, line in enumerate(f, 1):".to_string());
                lines.push(format!(
                    "                if re.search(r\"{}\", line):",
                    escape_pattern(&signal.description)
                ));
                lines.push(
                    "                    violations.append(f\"{filepath}:{lineno}: {line.strip()}\")"
                        .to_string(),
                );
                lines.push(format!(
                    "    assert not violations, f\"{{len(violations)}} violation(s) for {}: {{violations[:5]}}\"",
                    rule.id
                ));
                lines.push(String::new());
            }
            "ast" => {
                lines.push(format!("def {test_name}(python_source_files):"));
                lines.push(format!(
                    "    \"\"\"Signal: {} (ast)\"\"\"\n    violations = []",
                    signal.description
                ));
                lines.push("    for filepath in python_source_files:".to_string());
                lines.push("        with open(filepath) as f:".to_string());
                lines.push("            try:".to_string());
                lines.push(
                    "                tree = ast.parse(f.read(), filename=filepath)".to_string(),
                );
                lines.push("            except SyntaxError:".to_string());
                lines.push("                continue".to_string());
                lines.push("            for node in ast.walk(tree):".to_string());
                lines.push(format!(
                    "                pass  # TODO: implement AST check for: {}",
                    signal.description
                ));
                lines.push(format!(
                    "    assert not violations, f\"{{len(violations)}} violation(s) for {}\"",
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
    project_dir: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let warnings = Vec::new();

    let evals_dir = project_dir
        .join("whetstone")
        .join("evals")
        .join("typescript");

    // Generate setup.ts
    let setup = generate_ts_setup();
    let setup_path = evals_dir.join("setup.ts");
    if write_generated(&setup_path, &setup, dry_run) {
        test_files.push(serde_json::json!({
            "path": setup_path.display().to_string(),
            "type": "setup",
        }));
    }

    // Generate test file per dependency
    for rule in rules {
        let safe_name = rule
            .source_name
            .replace('@', "")
            .replace('/', "_")
            .replace('-', "_");
        let test_filename = format!("{safe_name}.test.ts");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_ts_test(rule);
        if write_generated(&test_path, &content, dry_run) {
            test_files.push(serde_json::json!({
                "path": test_path.display().to_string(),
                "type": "test",
                "rule_id": rule.id,
                "dependency": rule.source_name,
            }));
        }
    }

    // Generate biome overlay config
    let biome_rules = extract_lint_proxy_codes(rules, "biome");
    if !biome_rules.is_empty() {
        let biome_content = generate_biome_config(&biome_rules);
        let biome_path = project_dir
            .join("whetstone")
            .join("lint")
            .join("biome.whetstone.json");
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

fn generate_ts_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let _safe_id = rule
        .id
        .replace('.', "_")
        .replace('-', "_")
        .replace('@', "");

    lines.push("import { describe, it, expect } from 'vitest';".to_string());
    lines.push("import { findSourceFiles, readLines, violation } from './setup';".to_string());
    lines.push(String::new());

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
                lines.push("    for (const file of files) {".to_string());
                lines.push("      const lines = readLines(file);".to_string());
                lines.push("      lines.forEach((line, idx) => {".to_string());
                lines.push(format!(
                    "        // TODO: implement check for: {}",
                    signal.description
                ));
                lines.push("      });".to_string());
                lines.push("    }".to_string());
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
    project_dir: &Path,
    dry_run: bool,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut test_files = Vec::new();
    let mut lint_configs = Vec::new();
    let warnings = Vec::new();

    let evals_dir = project_dir.join("whetstone").join("evals").join("rust");

    for rule in rules {
        let safe_name = rule.source_name.replace('-', "_").replace('.', "_");
        let test_filename = format!("test_{safe_name}.rs");
        let test_path = evals_dir.join(&test_filename);

        let content = generate_rust_test(rule);
        if write_generated(&test_path, &content, dry_run) {
            test_files.push(serde_json::json!({
                "path": test_path.display().to_string(),
                "type": "test",
                "rule_id": rule.id,
                "dependency": rule.source_name,
            }));
        }
    }

    // Generate clippy overlay
    let clippy_rules = extract_lint_proxy_codes(rules, "clippy");
    if !clippy_rules.is_empty() {
        let clippy_content = generate_clippy_config(&clippy_rules);
        let clippy_path = project_dir
            .join("whetstone")
            .join("lint")
            .join("clippy.whetstone.toml");
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

fn generate_rust_test(rule: &ApprovedRule) -> String {
    let mut lines = Vec::new();
    let safe_id = rule.id.replace('.', "_").replace('-', "_");

    lines.push(format!("//! Whetstone eval: {}", rule.id));
    lines.push(format!("//! {}", rule.description.lines().next().unwrap_or("")));
    lines.push(String::new());
    lines.push("use std::fs;".to_string());
    lines.push("use std::path::Path;".to_string());
    lines.push(String::new());

    lines.push(format!(
        "fn find_rust_files(dir: &Path) -> Vec<std::path::PathBuf> {{"
    ));
    lines.push("    let mut files = Vec::new();".to_string());
    lines.push("    if let Ok(entries) = fs::read_dir(dir) {".to_string());
    lines.push("        for entry in entries.flatten() {".to_string());
    lines.push("            let path = entry.path();".to_string());
    lines.push("            if path.is_dir() {".to_string());
    lines.push("                let name = path.file_name().unwrap_or_default().to_string_lossy();".to_string());
    lines.push("                if !matches!(name.as_ref(), \"target\" | \".git\" | \"whetstone\") {".to_string());
    lines.push("                    files.extend(find_rust_files(&path));".to_string());
    lines.push("                }".to_string());
    lines.push("            } else if path.extension().and_then(|e| e.to_str()) == Some(\"rs\") {".to_string());
    lines.push("                files.push(path);".to_string());
    lines.push("            }".to_string());
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("    files".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

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
                lines.push(
                    "    let files = find_rust_files(Path::new(\"src\"));".to_string(),
                );
                lines.push("    let mut violations = Vec::new();".to_string());
                lines.push("    for file in &files {".to_string());
                lines.push(
                    "        if let Ok(content) = fs::read_to_string(file) {".to_string(),
                );
                lines.push(format!(
                    "            // TODO: implement check for: {}",
                    signal.description
                ));
                lines.push("            let _ = content;".to_string());
                lines.push("        }".to_string());
                lines.push("    }".to_string());
                lines.push(format!(
                    "    assert!(violations.is_empty(), \"{{}} violations for {}\", violations.len());",
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

fn escape_pattern(desc: &str) -> String {
    // Generate a basic regex pattern from the signal description
    // This is a best-effort heuristic
    desc.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('(', "\\(")
        .replace(')', "\\)")
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
