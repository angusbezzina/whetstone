//! Where the rulebook is thin: which parts of the repository (by top-level
//! directory and language) no rule in `whetstone/rulebook.json` reads, and
//! which rules read no file at all.
//!
//! A rule reads a file when its paths cover it and its enforcer reads that
//! kind of file: an AST query its language's files, a command its tool's
//! language when the rule names no paths, and a script or any other command
//! every file it covers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Value};

use crate::domain::{Enforcer, Rule};
use crate::rulebook::Rulebook;

fn language_of(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "css" | "scss" => "css",
        _ => "other",
    }
}

fn same_language(file: &str, rule: &str) -> bool {
    file == rule
        || (rule == "typescript" && file == "javascript")
        || (rule == "javascript" && file == "typescript")
}

/// The languages a command's tool reads, when it is one Whetstone knows.
fn tool_languages(command: &str) -> Option<&'static [&'static str]> {
    let program = command
        .split_whitespace()
        .map(|token| token.rsplit('/').next().unwrap_or(token))
        .find(|token| !token.contains('='))?;
    Some(match program {
        "cargo" | "rustfmt" | "clippy-driver" => &["rust"],
        "ruff" | "black" | "pytest" | "python" | "python3" | "mypy" | "flake8" | "tox" | "uv" => {
            &["python"]
        }
        "npm" | "npx" | "pnpm" | "yarn" | "bun" | "node" | "deno" | "eslint" | "prettier"
        | "biome" | "tsc" | "vitest" | "jest" => &["typescript", "javascript"],
        "go" | "gofmt" => &["go"],
        _ => return None,
    })
}

/// Whether `rule` reads the repository file at `path`.
pub fn reads(rule: &Rule, path: &str) -> bool {
    if !rule.applies_to(path) {
        return false;
    }
    let language = language_of(path);
    match &rule.enforcer {
        Enforcer::Ast {
            language: Some(wanted),
            ..
        } => same_language(language, wanted),
        Enforcer::Ast { language: None, .. } => {
            matches!(language, "rust" | "typescript" | "javascript" | "python")
        }
        Enforcer::Test { command }
        | Enforcer::Validator { command }
        | Enforcer::Formatter { tool: command }
        | Enforcer::Lint { tool: command, .. }
            if rule.paths.is_empty() && !rule.enforcer.is_script() =>
        {
            tool_languages(command).map_or(true, |languages| {
                languages
                    .iter()
                    .any(|wanted| same_language(language, wanted))
            })
        }
        Enforcer::Test { .. }
        | Enforcer::Validator { .. }
        | Enforcer::Formatter { .. }
        | Enforcer::Lint { .. } => true,
    }
}

/// Uncovered parts of the repository and rules that read nothing.
pub fn coverage(project_root: &Path, book: &Rulebook) -> Value {
    let files = crate::gates::FileSet::repository(project_root)
        .map(|files| {
            files
                .paths()
                .filter(|path| !crate::wire::is_generated(path))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut groups = BTreeMap::<(String, &'static str), Vec<String>>::new();
    let mut covered = 0usize;
    let mut reads_any = BTreeSet::new();
    for path in &files {
        let mut read = false;
        for entry in &book.rules {
            if reads(&entry.rule, path) {
                read = true;
                reads_any.insert(entry.id.as_str());
            }
        }
        if read {
            covered += 1;
            continue;
        }
        let directory = match path.split_once('/') {
            Some((top, _)) => format!("{top}/"),
            None => "(root)".to_string(),
        };
        groups
            .entry((directory, language_of(path)))
            .or_default()
            .push(path.clone());
    }
    let uncovered = groups
        .into_iter()
        .map(|((directory, language), paths)| {
            json!({
                "directory": directory,
                "language": language,
                "files": paths.len(),
                "sample": paths.iter().take(3).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let dead = book
        .rules
        .iter()
        .filter(|entry| !reads_any.contains(entry.id.as_str()))
        .map(|entry| {
            json!({
                "rule": entry.id.as_str(),
                "statement": entry.rule.statement,
                "paths": entry.rule.paths,
                "enforcer": entry.rule.enforcer.kind(),
                "why": if entry.rule.paths.is_empty() {
                    "its enforcer reads no file of a kind this repository has"
                } else {
                    "its paths match no file in the repository"
                },
            })
        })
        .collect::<Vec<_>>();
    json!({
        "files": files.len(),
        "covered": covered,
        "uncovered": uncovered,
        "dead_rules": dead,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(enforcer: Value, paths: &[&str]) -> Rule {
        serde_json::from_value(json!({
            "schema": "whetstone.rule.v2",
            "statement": "s",
            "rationale": "r",
            "strength": "must",
            "enforcer": enforcer,
            "source": {"kind": "owner"},
            "paths": paths,
        }))
        .expect("rule")
    }

    #[test]
    fn a_rule_reads_what_its_paths_and_enforcer_cover() {
        let rust_ast = rule(
            json!({"kind": "ast", "query": "(x) @match", "language": "rust"}),
            &[],
        );
        assert!(reads(&rust_ast, "src/main.rs"));
        assert!(!reads(&rust_ast, "web/app.ts"));
        let cargo = rule(json!({"kind": "test", "command": "cargo test"}), &[]);
        assert!(reads(&cargo, "src/main.rs"));
        assert!(!reads(&cargo, "docs/guide.md"));
        let scoped = rule(
            json!({"kind": "test", "command": "cargo test"}),
            &["docs/**"],
        );
        assert!(reads(&scoped, "docs/guide.md"));
        assert!(!reads(&scoped, "src/main.rs"));
        let script = rule(
            json!({"kind": "validator", "command": "sh whetstone/checks/x.sh"}),
            &[],
        );
        assert!(reads(&script, "docs/guide.md"));
        let unknown = rule(json!({"kind": "validator", "command": "make lint"}), &[]);
        assert!(reads(&unknown, "docs/guide.md"));
    }
}
