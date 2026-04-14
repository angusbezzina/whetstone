use crate::rules::{LoadedRuleFile, RuleFile};

/// Embedded built-in rule YAML.
const RUST_RULES: &str = include_str!("rust.yaml");
const PYTHON_RULES: &str = include_str!("python.yaml");
const TYPESCRIPT_RULES: &str = include_str!("typescript.yaml");

/// Load all built-in rules, returning parsed rule files.
pub fn load_builtin_rules() -> Vec<LoadedRuleFile> {
    let mut rules = Vec::new();

    for (label, language, text) in [
        ("builtin:rust", "rust", RUST_RULES),
        ("builtin:python", "python", PYTHON_RULES),
        ("builtin:typescript", "typescript", TYPESCRIPT_RULES),
    ] {
        match serde_yaml::from_str::<RuleFile>(text) {
            Ok(rf) => rules.push(LoadedRuleFile {
                file_path: label.to_string(),
                language: Some(language.to_string()),
                rule_file: rf,
            }),
            Err(e) => {
                eprintln!(
                    "Warning: failed to parse built-in {label} rules: {e} — ignoring built-in set"
                );
            }
        }
    }

    rules
}

/// Merge built-in LoadedRuleFiles with project LoadedRuleFiles.
/// Used by status pipeline (JSON-based). Project overrides built-in by ID.
#[allow(dead_code)]
pub fn merge_rules(
    builtin: &[LoadedRuleFile],
    project: &[LoadedRuleFile],
    deny: &[String],
) -> Vec<LoadedRuleFile> {
    use std::collections::HashSet;

    // Collect project rule IDs for override detection
    let project_rule_ids: HashSet<String> = project
        .iter()
        .flat_map(|lrf| lrf.rule_file.rules.iter().map(|r| r.id.clone()))
        .collect();

    let deny_set: HashSet<&str> = deny.iter().map(|s| s.as_str()).collect();

    let mut merged = Vec::new();

    // Add built-in rules that aren't overridden by project rules or denied
    for lrf in builtin {
        let filtered_rules: Vec<_> = lrf
            .rule_file
            .rules
            .iter()
            .filter(|r| !project_rule_ids.contains(&r.id) && !deny_set.contains(r.id.as_str()))
            .cloned()
            .collect();

        if !filtered_rules.is_empty() {
            merged.push(LoadedRuleFile {
                file_path: lrf.file_path.clone(),
                language: lrf.language.clone(),
                rule_file: RuleFile {
                    source: lrf.rule_file.source.clone(),
                    rules: filtered_rules,
                },
            });
        }
    }

    // Add all project rules (minus denied)
    for lrf in project {
        let filtered_rules: Vec<_> = lrf
            .rule_file
            .rules
            .iter()
            .filter(|r| !deny_set.contains(r.id.as_str()))
            .cloned()
            .collect();

        if !filtered_rules.is_empty() {
            merged.push(LoadedRuleFile {
                file_path: lrf.file_path.clone(),
                language: lrf.language.clone(),
                rule_file: RuleFile {
                    source: lrf.rule_file.source.clone(),
                    rules: filtered_rules,
                },
            });
        }
    }

    merged
}
