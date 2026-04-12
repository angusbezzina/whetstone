use crate::rules::{LoadedRuleFile, RuleFile};

/// Embedded built-in rule YAML for Rust.
const RUST_RULES: &str = include_str!("rust.yaml");

/// Load all built-in rules, returning parsed rule files.
pub fn load_builtin_rules() -> Vec<LoadedRuleFile> {
    let mut rules = Vec::new();

    if let Ok(rf) = serde_yaml::from_str::<RuleFile>(RUST_RULES) {
        rules.push(LoadedRuleFile {
            file_path: "builtin:rust".to_string(),
            language: Some("rust".to_string()),
            rule_file: rf,
        });
    }

    // Future: add Python, TypeScript built-in rules here
    // const PYTHON_RULES: &str = include_str!("python.yaml");
    // const TYPESCRIPT_RULES: &str = include_str!("typescript.yaml");

    rules
}

/// Merge built-in ApprovedRules with project ApprovedRules.
/// Project rules override built-in by ID. Denied IDs excluded.
pub fn merge_approved_rules(
    builtin: &[crate::rules::ApprovedRule],
    project: &[crate::rules::ApprovedRule],
    deny: &[String],
) -> Vec<crate::rules::ApprovedRule> {
    use std::collections::HashSet;

    let project_ids: HashSet<&str> = project.iter().map(|r| r.id.as_str()).collect();
    let deny_set: HashSet<&str> = deny.iter().map(|s| s.as_str()).collect();

    let mut merged: Vec<crate::rules::ApprovedRule> = Vec::new();

    // Add built-in rules not overridden and not denied
    for rule in builtin {
        if !project_ids.contains(rule.id.as_str()) && !deny_set.contains(rule.id.as_str()) {
            // Clone by reconstructing (ApprovedRule doesn't derive Clone)
            merged.push(crate::rules::ApprovedRule {
                id: rule.id.clone(),
                severity: rule.severity.clone(),
                confidence: rule.confidence.clone(),
                category: rule.category.clone(),
                description: rule.description.clone(),
                source_url: rule.source_url.clone(),
                source_name: rule.source_name.clone(),
                language: rule.language.clone(),
                signals: rule.signals.iter().map(|s| crate::rules::ApprovedSignal {
                    id: s.id.clone(),
                    strategy: s.strategy.clone(),
                    description: s.description.clone(),
                    weight: s.weight.clone(),
                    match_pattern: s.match_pattern.clone(),
                }).collect(),
                golden_examples: rule.golden_examples.iter().map(|e| crate::rules::ApprovedExample {
                    code: e.code.clone(),
                    verdict: e.verdict.clone(),
                    reason: e.reason.clone(),
                    language: e.language.clone(),
                }).collect(),
                risk: rule.risk.clone(),
                linter_gap: rule.linter_gap.clone(),
            });
        }
    }

    // Add all project rules not denied
    for rule in project {
        if !deny_set.contains(rule.id.as_str()) {
            merged.push(crate::rules::ApprovedRule {
                id: rule.id.clone(),
                severity: rule.severity.clone(),
                confidence: rule.confidence.clone(),
                category: rule.category.clone(),
                description: rule.description.clone(),
                source_url: rule.source_url.clone(),
                source_name: rule.source_name.clone(),
                language: rule.language.clone(),
                signals: rule.signals.iter().map(|s| crate::rules::ApprovedSignal {
                    id: s.id.clone(),
                    strategy: s.strategy.clone(),
                    description: s.description.clone(),
                    weight: s.weight.clone(),
                    match_pattern: s.match_pattern.clone(),
                }).collect(),
                golden_examples: rule.golden_examples.iter().map(|e| crate::rules::ApprovedExample {
                    code: e.code.clone(),
                    verdict: e.verdict.clone(),
                    reason: e.reason.clone(),
                    language: e.language.clone(),
                }).collect(),
                risk: rule.risk.clone(),
                linter_gap: rule.linter_gap.clone(),
            });
        }
    }

    merged
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
