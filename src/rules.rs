//! Structured YAML rule loading and validation.
//!
//! Replaces the regex-based rule parsing with serde_yaml deserialization
//! and provides validation against the rule schema.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Valid rule categories per project spec.
const VALID_CATEGORIES: &[&str] = &[
    "migration",
    "default",
    "convention",
    "breaking-change",
    "semantic",
];

/// Valid signal strategies.
const VALID_STRATEGIES: &[&str] = &["ast", "pattern", "lint_proxy", "ai"];

/// Valid severity levels.
const VALID_SEVERITIES: &[&str] = &["must", "should", "may"];

/// Valid confidence levels.
const VALID_CONFIDENCES: &[&str] = &["high", "medium"];

// --- Serde deserialization types ---

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct RuleFile {
    #[serde(default)]
    pub source: RuleSource,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct RuleSource {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub llms_txt: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_quote: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub linter_gap: Option<String>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub proposed_at: Option<String>,
    #[serde(default)]
    pub proposed_by: Option<String>,
    #[serde(default)]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub golden_examples: Vec<GoldenExample>,
}

#[derive(Debug, Deserialize)]
pub struct Signal {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub weight: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoldenExample {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

// --- Validation ---

#[allow(dead_code)]
pub struct ValidationWarning {
    pub file: String,
    pub message: String,
}

/// Validate a parsed rule file against the schema invariants.
pub fn validate_rule_file(rf: &RuleFile, file_path: &str) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    if rf.source.name.is_empty() {
        warnings.push(ValidationWarning {
            file: file_path.to_string(),
            message: "source.name is empty".to_string(),
        });
    }

    for rule in &rf.rules {
        let rule_ctx = if rule.id.is_empty() {
            format!("{file_path}: unnamed rule")
        } else {
            format!("{file_path}: rule {}", rule.id)
        };

        if rule.id.is_empty() {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing id"),
            });
        }

        if let Some(ref sev) = rule.severity {
            if !VALID_SEVERITIES.contains(&sev.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: invalid severity '{sev}'"),
                });
            }
        } else {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing severity"),
            });
        }

        if let Some(ref conf) = rule.confidence {
            if !VALID_CONFIDENCES.contains(&conf.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: invalid confidence '{conf}'"),
                });
            }
        } else {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing confidence"),
            });
        }

        if let Some(ref cat) = rule.category {
            if !VALID_CATEGORIES.contains(&cat.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: invalid category '{cat}'"),
                });
            }
        } else {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing category"),
            });
        }

        if rule.description.is_none() {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing description"),
            });
        }

        if rule.source_url.is_none() {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: missing source_url"),
            });
        }

        // Must have at least one deterministic signal
        let has_deterministic = rule
            .signals
            .iter()
            .any(|s| matches!(s.strategy.as_str(), "ast" | "pattern"));
        if !has_deterministic && !rule.signals.is_empty() {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: no deterministic signal (ast or pattern)"),
            });
        }

        for sig in &rule.signals {
            if !VALID_STRATEGIES.contains(&sig.strategy.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: signal has invalid strategy '{}'", sig.strategy),
                });
            }
        }

        // Golden examples: should have 3-5
        if rule.golden_examples.is_empty() && rule.approved {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: approved rule has no golden_examples"),
            });
        }
    }

    warnings
}

// --- Loading ---

/// Load all rule files from the rules directory, returning parsed files and warnings.
pub fn load_rule_files(rules_dir: &Path) -> (Vec<LoadedRuleFile>, Vec<String>) {
    let mut rule_files = Vec::new();
    let mut warnings = Vec::new();

    if !rules_dir.exists() {
        return (rule_files, warnings);
    }

    for entry in walkdir::WalkDir::new(rules_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }

        let text = match std::fs::read_to_string(entry.path()) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("Failed to read {}: {e}", entry.path().display()));
                continue;
            }
        };

        match serde_yaml::from_str::<RuleFile>(&text) {
            Ok(rf) => {
                let file_path = entry.path().to_string_lossy().to_string();

                // Validate
                let validation = validate_rule_file(&rf, &file_path);
                for w in &validation {
                    warnings.push(w.message.clone());
                }

                // Infer language from path
                let language = infer_language_from_path(entry.path(), rules_dir);

                rule_files.push(LoadedRuleFile {
                    file_path,
                    rule_file: rf,
                    language,
                });
            }
            Err(e) => {
                warnings.push(format!(
                    "Failed to parse YAML {}: {e}",
                    entry.path().display()
                ));
            }
        }
    }

    (rule_files, warnings)
}

/// Infer language from the directory structure (e.g., rules/python/fastapi.yaml → python).
fn infer_language_from_path(file_path: &Path, rules_dir: &Path) -> Option<String> {
    if let Ok(relative) = file_path.strip_prefix(rules_dir) {
        let components: Vec<_> = relative.components().collect();
        if components.len() >= 2 {
            let dir = components[0].as_os_str().to_string_lossy().to_string();
            if matches!(dir.as_str(), "python" | "typescript" | "rust") {
                return Some(dir);
            }
        }
    }
    None
}

/// A loaded rule file with metadata.
pub struct LoadedRuleFile {
    pub file_path: String,
    pub rule_file: RuleFile,
    pub language: Option<String>,
}

/// Convert loaded rule files into the JSON format used by status.rs.
pub fn rule_files_to_json(loaded: &[LoadedRuleFile]) -> Vec<Value> {
    loaded
        .iter()
        .map(|lrf| {
            let rf = &lrf.rule_file;
            let rules_json: Vec<Value> = rf
                .rules
                .iter()
                .map(|r| {
                    let signal_strategies: Vec<String> =
                        r.signals.iter().map(|s| s.strategy.clone()).collect();
                    serde_json::json!({
                        "id": r.id,
                        "severity": r.severity,
                        "confidence": r.confidence,
                        "category": r.category,
                        "description": r.description,
                        "source_url": r.source_url,
                        "approved": r.approved,
                        "approved_at": r.approved_at,
                        "signals": signal_strategies,
                        "status": r.status,
                    })
                })
                .collect();

            serde_json::json!({
                "file": lrf.file_path,
                "source_name": rf.source.name,
                "source_version": rf.source.version,
                "content_hash": rf.source.content_hash,
                "language": lrf.language,
                "rules": rules_json,
            })
        })
        .collect()
}

/// Load rules and return the same (Vec<Value>, Vec<String>) format as the old regex-based loader.
pub fn load_rules_as_json(rules_dir: &Path) -> (Vec<Value>, Vec<String>) {
    let (loaded, warnings) = load_rule_files(rules_dir);
    (rule_files_to_json(&loaded), warnings)
}

/// Load only approved rules, optionally filtered by language.
pub fn load_approved_rules(
    rules_dir: &Path,
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let (loaded, warnings) = load_rule_files(rules_dir);
    let mut approved = Vec::new();

    for lrf in &loaded {
        let language = lrf.language.as_deref().unwrap_or("generic");

        if let Some(filter) = lang_filter {
            if language != filter {
                continue;
            }
        }

        for rule in &lrf.rule_file.rules {
            if !rule.approved {
                continue;
            }

            approved.push(ApprovedRule {
                id: rule.id.clone(),
                severity: rule.severity.clone().unwrap_or_default(),
                confidence: rule.confidence.clone().unwrap_or_default(),
                category: rule.category.clone().unwrap_or_default(),
                description: rule.description.clone().unwrap_or_default(),
                source_url: rule.source_url.clone().unwrap_or_default(),
                source_name: lrf.rule_file.source.name.clone(),
                language: language.to_string(),
                signals: rule
                    .signals
                    .iter()
                    .map(|s| ApprovedSignal {
                        id: s.id.clone().unwrap_or_default(),
                        strategy: s.strategy.clone(),
                        description: s.description.clone().unwrap_or_default(),
                        weight: s.weight.clone().unwrap_or_default(),
                    })
                    .collect(),
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
                risk: rule.risk.clone(),
                linter_gap: rule.linter_gap.clone(),
            });
        }
    }

    (approved, warnings)
}

#[allow(dead_code)]
pub struct ApprovedRule {
    pub id: String,
    pub severity: String,
    pub confidence: String,
    pub category: String,
    pub description: String,
    pub source_url: String,
    pub source_name: String,
    pub language: String,
    pub signals: Vec<ApprovedSignal>,
    pub golden_examples: Vec<ApprovedExample>,
    pub risk: Option<String>,
    pub linter_gap: Option<String>,
}

#[allow(dead_code)]
pub struct ApprovedSignal {
    pub id: String,
    pub strategy: String,
    pub description: String,
    pub weight: String,
}

#[allow(dead_code)]
pub struct ApprovedExample {
    pub code: String,
    pub verdict: String,
    pub reason: String,
    pub language: Option<String>,
}

/// Collect summary stats from loaded rule files (used by multiple commands).
#[allow(dead_code)]
pub fn compute_rule_stats(rule_files: &[Value]) -> BTreeMap<String, Value> {
    let mut stats = BTreeMap::new();

    let mut total_rules = 0usize;
    let mut approved_count = 0usize;
    let mut dep_names: Vec<String> = Vec::new();

    for rf in rule_files {
        if let Some(name) = rf.get("source_name").and_then(|v| v.as_str()) {
            dep_names.push(name.to_string());
        }
        if let Some(rules) = rf.get("rules").and_then(|v| v.as_array()) {
            total_rules += rules.len();
            approved_count += rules
                .iter()
                .filter(|r| r.get("approved").and_then(|v| v.as_bool()).unwrap_or(false))
                .count();
        }
    }

    stats.insert("total_rules".to_string(), Value::from(total_rules));
    stats.insert("approved_count".to_string(), Value::from(approved_count));
    stats.insert(
        "dependencies".to_string(),
        Value::Array(dep_names.iter().map(|n| Value::String(n.clone())).collect()),
    );

    stats
}
