//! Structured YAML rule loading and validation.
//!
//! Provides typed rule loading and validation for the deterministic kernel.

use serde::{Deserialize, Serialize};
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
const VALID_STRATEGIES: &[&str] = &["ast", "pattern", "lint_proxy"];

/// Valid severity levels.
const VALID_SEVERITIES: &[&str] = &["must", "should", "may"];

/// Valid confidence levels.
const VALID_CONFIDENCES: &[&str] = &["high", "medium"];

/// Valid rule lifecycle statuses in the retained rule schema.
const VALID_STATUSES: &[&str] = &["candidate", "approved"];

/// Supported lint tools for structured lint bindings.
const VALID_LINT_TOOLS: &[&str] = &["ruff", "biome", "clippy"];

/// Supported formatter tools for formatter-backed custom rules.
const VALID_FORMATTER_TOOLS: &[&str] = &["ruff", "biome", "rustfmt"];

/// Supported explicit test runners for test-backed custom rules.
const VALID_TEST_RUNNERS: &[&str] = &["pytest", "vitest", "cargo"];

/// Supported validator modes for adapter-backed rules.
const VALID_VALIDATOR_MODES: &[&str] = &["enforce", "monitor"];

/// Supported validator adapters.
const VALID_VALIDATOR_ADAPTERS: &[&str] = &["command", "lint_rule", "linked_test"];

// --- Serde deserialization types ---

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct RuleFile {
    #[serde(default)]
    pub source: RuleSource,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
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
    /// How the binary fetched content: llms_txt, readme, html_converted, changelog, custom_url
    #[serde(default)]
    pub content_origin: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
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
    pub approved: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub deterministic_pass_threshold: Option<u32>,
    #[serde(default)]
    pub deterministic_fail_threshold: Option<u32>,
    #[serde(default)]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub formatter: Option<FormatterDirective>,
    #[serde(default)]
    pub tests: Vec<TestBinding>,
    #[serde(default)]
    pub validators: Vec<ValidatorBinding>,
    #[serde(default)]
    pub provenance: Option<RuleProvenance>,
    #[serde(default)]
    pub golden_examples: Vec<GoldenExample>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormatterDirective {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LintBinding {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub code: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestBinding {
    #[serde(default)]
    pub runner: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub selector: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ValidatorBinding {
    #[serde(default)]
    pub adapter: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub config: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleProvenance {
    #[serde(default)]
    pub source_page_id: Option<String>,
    #[serde(default)]
    pub source_page_path: Option<String>,
    #[serde(default)]
    pub source_authority: Option<String>,
    #[serde(default)]
    pub source_line_start: Option<u32>,
    #[serde(default)]
    pub source_line_end: Option<u32>,
    #[serde(default)]
    pub upstream_urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Signal {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub weight: Option<String>,
    /// Raw tree-sitter query (S-expression) for `ast` signals. Every node
    /// captured by `@match` is reported as a violation. When present, the
    /// scanner uses the project's tree-sitter grammar for the rule language.
    #[serde(default)]
    pub ast_query: Option<String>,
    /// Structured lint binding for `lint_proxy` signals. Prefer this over
    /// encoding the tool/code pair in free-text descriptions.
    #[serde(default)]
    pub lint: Option<LintBinding>,
}

#[derive(Debug, Clone, Deserialize)]
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

        let has_enforcement_surface = rule
            .signals
            .iter()
            .any(|s| matches!(s.strategy.as_str(), "ast" | "pattern" | "lint_proxy"))
            || rule.formatter.is_some()
            || !rule.tests.is_empty()
            || !rule.validators.is_empty();
        if !has_enforcement_surface {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!(
                    "{rule_ctx}: no enforceable signal or binding (expected ast, pattern, lint_proxy, formatter, tests, or validators)"
                ),
            });
        }

        if rule.signals.is_empty()
            && rule.formatter.is_none()
            && rule.tests.is_empty()
            && rule.validators.is_empty()
        {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: no signals, formatter, tests, or validators defined"),
            });
        }

        for sig in &rule.signals {
            if !VALID_STRATEGIES.contains(&sig.strategy.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: signal has invalid strategy '{}'", sig.strategy),
                });
            }

            if sig.strategy == "pattern" {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: signal `{}` uses non-executable legacy `strategy: pattern`; migrate to `ast` with ast_query or `lint_proxy`",
                        sig.id.clone().unwrap_or_default()
                    ),
                });
            }

            if sig.strategy == "ast" && sig.ast_query.is_none() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: signal `{}` is `ast` but has no ast_query",
                        sig.id.clone().unwrap_or_default()
                    ),
                });
            }

            if sig.weight.as_deref().map_or(true, str::is_empty) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: signal is missing weight"),
                });
            }

            if let Some(lint) = &sig.lint {
                if sig.strategy != "lint_proxy" {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: signal `{}` declares `lint` metadata but strategy is `{}` (expected lint_proxy)",
                            sig.id.clone().unwrap_or_default(),
                            sig.strategy
                        ),
                    });
                }
                if lint.tool.trim().is_empty() {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: signal `{}` has empty lint.tool",
                            sig.id.clone().unwrap_or_default()
                        ),
                    });
                } else if !VALID_LINT_TOOLS.contains(&lint.tool.as_str()) {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: signal `{}` uses unsupported lint.tool `{}`",
                            sig.id.clone().unwrap_or_default(),
                            lint.tool
                        ),
                    });
                }
                if lint.code.trim().is_empty() {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: signal `{}` has empty lint.code",
                            sig.id.clone().unwrap_or_default()
                        ),
                    });
                }
            } else if sig.strategy == "lint_proxy" {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: signal `{}` uses legacy lint_proxy description parsing; add lint.tool and lint.code",
                        sig.id.clone().unwrap_or_default()
                    ),
                });
            }
        }

        for example in &rule.golden_examples {
            if example.reason.as_deref().map_or(true, str::is_empty) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: golden example is missing reason"),
                });
            }
        }

        if let Some(formatter) = &rule.formatter {
            if formatter.tool.trim().is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: formatter.tool is empty"),
                });
            } else if !VALID_FORMATTER_TOOLS.contains(&formatter.tool.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: formatter.tool `{}` is unsupported",
                        formatter.tool
                    ),
                });
            }
            if formatter.options.is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: formatter.options is empty"),
                });
            }
        }

        for test in &rule.tests {
            if test.runner.trim().is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: tests[].runner is empty"),
                });
            } else if !VALID_TEST_RUNNERS.contains(&test.runner.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: tests[].runner `{}` is unsupported",
                        test.runner
                    ),
                });
            }
            if test.path.trim().is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: tests[].path is empty"),
                });
            } else if !is_safe_repo_relative(&test.path) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: tests[].path must stay within the repo"),
                });
            }
        }

        for validator in &rule.validators {
            if validator.adapter.trim().is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: validators[].adapter is empty"),
                });
            } else if !VALID_VALIDATOR_ADAPTERS.contains(&validator.adapter.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: validators[].adapter `{}` is unsupported",
                        validator.adapter
                    ),
                });
            }
            if validator.rule.trim().is_empty() {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: validators[].rule is empty"),
                });
            }
            if let Some(mode) = &validator.mode {
                if !VALID_VALIDATOR_MODES.contains(&mode.as_str()) {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!("{rule_ctx}: validators[].mode `{mode}` is unsupported"),
                    });
                }
            }
            match validator.adapter.as_str() {
                "command" => {
                    let has_path = validator
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .is_some();
                    let has_command = validator
                        .config
                        .get("command")
                        .and_then(|v| v.as_str())
                        .is_some();
                    if !has_path && !has_command {
                        warnings.push(ValidationWarning {
                            file: file_path.to_string(),
                            message: format!(
                                "{rule_ctx}: command validator requires config.path or config.command"
                            ),
                        });
                    }
                    if let Some(path) = validator.config.get("path").and_then(|v| v.as_str()) {
                        if !is_safe_repo_relative(path) {
                            warnings.push(ValidationWarning {
                                file: file_path.to_string(),
                                message: format!(
                                    "{rule_ctx}: command validator path must stay within the repo"
                                ),
                            });
                        }
                    }
                    if has_command
                        && validator
                            .config
                            .get("allow_shell")
                            .and_then(|v| v.as_bool())
                            != Some(true)
                    {
                        warnings.push(ValidationWarning {
                            file: file_path.to_string(),
                            message: format!(
                                "{rule_ctx}: command validator using config.command requires config.allow_shell=true"
                            ),
                        });
                    }
                }
                "lint_rule" => {
                    if validator
                        .config
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || validator
                            .config
                            .get("code")
                            .and_then(|v| v.as_str())
                            .is_none()
                    {
                        warnings.push(ValidationWarning {
                            file: file_path.to_string(),
                            message: format!(
                                "{rule_ctx}: lint_rule validator requires config.tool and config.code"
                            ),
                        });
                    }
                }
                "linked_test" => {
                    if validator
                        .config
                        .get("runner")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || validator
                            .config
                            .get("path")
                            .and_then(|v| v.as_str())
                            .is_none()
                    {
                        warnings.push(ValidationWarning {
                            file: file_path.to_string(),
                            message: format!(
                                "{rule_ctx}: linked_test validator requires config.runner and config.path"
                            ),
                        });
                    }
                    if let Some(path) = validator.config.get("path").and_then(|v| v.as_str()) {
                        if !is_safe_repo_relative(path) {
                            warnings.push(ValidationWarning {
                                file: file_path.to_string(),
                                message: format!(
                                    "{rule_ctx}: linked_test validator path must stay within the repo"
                                ),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Golden examples: should have 3-5
        if rule.golden_examples.is_empty() && rule.approved {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: approved rule has no golden_examples"),
            });
        }

        // Lifecycle status transitions
        if let Some(ref status) = rule.status {
            if !VALID_STATUSES.contains(&status.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!(
                        "{rule_ctx}: invalid status '{status}' (expected one of: {})",
                        VALID_STATUSES.join(", ")
                    ),
                });
            } else {
                // Consistency: the `approved` boolean and the `status` enum must agree.
                let status_s = status.as_str();
                if rule.approved && status_s != "approved" {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: approved=true but status='{status}' (expected 'approved')"
                        ),
                    });
                }
                if !rule.approved && status_s == "approved" {
                    warnings.push(ValidationWarning {
                        file: file_path.to_string(),
                        message: format!(
                            "{rule_ctx}: status='approved' but approved=false — set approved: true or change status to candidate"
                        ),
                    });
                }
            }
        } else if rule.approved {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: approved=true requires explicit status='approved'"),
            });
        }
    }

    warnings
}

// --- Schema + rule-file validation ---

/// Required field names the schema file must document.
const SCHEMA_REQUIRED_FIELDS: &[&str] = &[
    "id",
    "severity",
    "confidence",
    "category",
    "description",
    "source_url",
    "signals",
];

/// The rule-schema YAML embedded at compile time so `wh validate` works for
/// downstream projects that do not carry the Whetstone source tree.
const EMBEDDED_SCHEMA: &str = include_str!("../references/rule-schema.yaml");

/// Validate the schema file and all rule fixtures under the given project root.
///
/// Produces a human-readable report and a boolean indicating overall success.
/// Used by the hidden `validate` command and the CI schema gate.
pub fn validate_schema_and_fixtures(project_root: &Path) -> (String, bool) {
    let mut out = String::new();
    let mut ok = true;

    // Prefer the project-local schema (so Whetstone itself + forks are
    // self-validating), but fall back to the binary-embedded schema for
    // external projects that only have rule files — not the whole Whetstone
    // source tree.
    let schema_path = project_root.join("references").join("rule-schema.yaml");
    let schema_text = if schema_path.exists() {
        match std::fs::read_to_string(&schema_path) {
            Ok(t) => {
                out.push_str("Schema file found and readable.\n");
                t
            }
            Err(e) => {
                out.push_str(&format!(
                    "FAIL: cannot read {}: {e}\n",
                    schema_path.display()
                ));
                return (out, false);
            }
        }
    } else {
        out.push_str("Schema file (project-local) not found — using binary-embedded schema.\n");
        EMBEDDED_SCHEMA.to_string()
    };
    for field in SCHEMA_REQUIRED_FIELDS {
        if !schema_text.contains(field) {
            out.push_str(&format!(
                "FAIL: required field \"{field}\" not found in schema\n"
            ));
            ok = false;
        } else {
            out.push_str(&format!("  OK: {field}\n"));
        }
    }
    if !ok {
        return (out, false);
    }

    // Collect YAML files from every layer that can carry rules: test
    // fixtures, project rules, and the personal layer (local-only
    // overrides) — so `wh validate` catches schema drift everywhere.
    let scan_roots = [
        project_root.join("tests").join("fixtures"),
        project_root.join("whetstone").join("rules"),
        project_root
            .join("whetstone")
            .join(".personal")
            .join("rules"),
    ];
    let mut fixtures: Vec<std::path::PathBuf> = Vec::new();
    for root in &scan_roots {
        if root.exists() {
            for entry in walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
                    fixtures.push(entry.path().to_path_buf());
                }
            }
        }
    }
    fixtures.sort();
    out.push_str(&format!("Checking {} rule files...\n", fixtures.len()));

    let mut errors: Vec<String> = Vec::new();
    for fixture in &fixtures {
        let rel = fixture
            .strip_prefix(project_root)
            .unwrap_or(fixture)
            .to_string_lossy()
            .replace('\\', "/");
        let text = match std::fs::read_to_string(fixture) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("{rel}: read error: {e}"));
                continue;
            }
        };

        let rf: RuleFile = match serde_yaml::from_str(&text) {
            Ok(rf) => rf,
            Err(e) => {
                errors.push(format!("{rel}: parse error: {e}"));
                continue;
            }
        };

        for warning in validate_rule_file(&rf, &rel) {
            errors.push(warning.message);
        }

        if rf.rules.is_empty() {
            continue;
        }

        for rule in &rf.rules {
            let rid = if rule.id.is_empty() { "?" } else { &rule.id };
            for req in ["id", "severity", "confidence", "category", "source_url"] {
                let missing = match req {
                    "id" => rule.id.is_empty(),
                    "severity" => rule.severity.is_none(),
                    "confidence" => rule.confidence.is_none(),
                    "category" => rule.category.is_none(),
                    "source_url" => rule.source_url.is_none(),
                    _ => false,
                };
                if missing {
                    errors.push(format!("{rel}: rule {rid} missing {req}"));
                }
            }

            if let Some(ref sev) = rule.severity {
                if !VALID_SEVERITIES.contains(&sev.as_str()) {
                    errors.push(format!("{rel}: rule {rid} invalid severity \"{sev}\""));
                }
            }
            if let Some(ref conf) = rule.confidence {
                if !VALID_CONFIDENCES.contains(&conf.as_str()) {
                    errors.push(format!("{rel}: rule {rid} invalid confidence \"{conf}\""));
                }
            }
            if let Some(ref cat) = rule.category {
                if !VALID_CATEGORIES.contains(&cat.as_str()) {
                    errors.push(format!("{rel}: rule {rid} invalid category \"{cat}\""));
                }
            }
            for sig in &rule.signals {
                if !VALID_STRATEGIES.contains(&sig.strategy.as_str()) {
                    errors.push(format!(
                        "{rel}: rule {rid} invalid strategy \"{}\"",
                        sig.strategy
                    ));
                }

                // Pattern records remain parseable only for migration. They are
                // never executable in the lean kernel.
                if sig.strategy == "pattern" {
                    let msg = format!(
                        "{rel}: rule {rid} uses non-executable legacy `strategy: pattern`; migrate to `strategy: ast` with ast_query or `strategy: lint_proxy`"
                    );
                    if rel.starts_with("whetstone/rules/") {
                        errors.push(msg);
                    } else {
                        out.push_str(&format!("  WARN: {msg}\n"));
                    }
                }

                // The lean kernel never weakens an AST rule to text matching.
                if sig.strategy == "ast" && sig.ast_query.is_none() {
                    errors.push(format!(
                        "{rel}: rule {rid} `ast` signal is missing ast_query"
                    ));
                }

                if sig.strategy == "ast" {
                    let inferred_language = infer_language_from_rule_path(fixture);
                    let languages = resolved_rule_languages(rule, inferred_language.as_deref());
                    for language in languages {
                        match (
                            crate::ast::AstLang::from_str(&language),
                            sig.ast_query.as_deref(),
                        ) {
                            (Some(ast_lang), Some(query)) => {
                                if let Err(error) = crate::ast::compile_query(ast_lang, query) {
                                    errors.push(format!("{rel}: rule {rid}: {error}"));
                                }
                            }
                            (None, Some(_)) => errors.push(format!(
                                "{rel}: rule {rid}: ast strategy is unsupported for language {language}"
                            )),
                            _ => {}
                        }
                    }
                }
            }

            out.push_str(&format!("  OK: {rel} / {rid}\n"));
        }
    }

    if !errors.is_empty() {
        for e in &errors {
            out.push_str(&format!("FAIL: {e}\n"));
        }
        return (out, false);
    }

    out.push_str("All schema checks passed.\n");
    (out, true)
}

// --- Loading ---

/// Load valid rule files from the rules directory, returning rejected-file issues.
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

                let validation = validate_rule_file(&rf, &file_path);
                if !validation.is_empty() {
                    warnings.extend(validation.into_iter().map(|warning| warning.message));
                    continue;
                }

                // Infer language from path
                let language = infer_language_from_path(entry.path(), rules_dir);

                rule_files.push(LoadedRuleFile {
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
            if crate::types::canonical_language(&dir).is_some()
                || dir == crate::types::SHARED_LANGUAGE_DIR
            {
                return Some(dir);
            }
        }
    }
    None
}

fn infer_language_from_rule_path(file_path: &Path) -> Option<String> {
    let components: Vec<_> = file_path.components().collect();
    for pair in components.windows(2) {
        if pair[0].as_os_str() != "rules" {
            continue;
        }
        let language = pair[1].as_os_str().to_string_lossy().to_string();
        if crate::types::canonical_language(&language).is_some()
            || language == crate::types::SHARED_LANGUAGE_DIR
        {
            return Some(language);
        }
    }
    None
}

fn is_safe_repo_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

/// A loaded rule file with metadata.
pub struct LoadedRuleFile {
    pub rule_file: RuleFile,
    pub language: Option<String>,
}

/// Load only approved rules, optionally filtered by language.
pub fn load_approved_rules(
    rules_dir: &Path,
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let (loaded, warnings) = load_rule_files(rules_dir);
    let mut approved = Vec::new();

    for lrf in &loaded {
        for rule in &lrf.rule_file.rules {
            if !rule.approved || rule.status.as_deref() != Some("approved") {
                continue;
            }
            let target_languages = resolved_rule_languages(rule, lrf.language.as_deref());
            if target_languages.is_empty() {
                continue;
            }
            for language in target_languages_for_filter(&target_languages, lang_filter) {
                approved.push(ApprovedRule {
                    id: rule.id.clone(),
                    severity: rule.severity.clone().unwrap_or_default(),
                    category: rule.category.clone().unwrap_or_default(),
                    description: rule.description.clone().unwrap_or_default(),
                    source_url: rule.source_url.clone().unwrap_or_default(),
                    source_name: lrf.rule_file.source.name.clone(),
                    language,
                    signals: rule
                        .signals
                        .iter()
                        .map(|s| ApprovedSignal {
                            id: s.id.clone().unwrap_or_default(),
                            strategy: s.strategy.clone(),
                            description: s.description.clone().unwrap_or_default(),
                            ast_query: s.ast_query.clone(),
                            lint: s.lint.as_ref().map(|lint| ApprovedLintBinding {
                                tool: lint.tool.clone(),
                                code: lint.code.clone(),
                            }),
                        })
                        .collect(),
                    formatter: rule.formatter.as_ref().map(|f| ApprovedFormatterDirective {
                        tool: f.tool.clone(),
                        options: f.options.clone(),
                    }),
                    tests: rule
                        .tests
                        .iter()
                        .map(|t| ApprovedTestBinding {
                            runner: t.runner.clone(),
                            path: t.path.clone(),
                            selector: t.selector.clone(),
                        })
                        .collect(),
                    validators: rule
                        .validators
                        .iter()
                        .map(|validator| ApprovedValidatorBinding {
                            adapter: validator.adapter.clone(),
                            rule: validator.rule.clone(),
                            config: validator.config.clone(),
                        })
                        .collect(),
                    golden_examples: rule
                        .golden_examples
                        .iter()
                        .map(|e| ApprovedExample {
                            code: e.code.clone(),
                            verdict: e.verdict.clone(),
                            language: e.language.clone(),
                        })
                        .collect(),
                });
            }
        }
    }

    (approved, warnings)
}

fn resolved_rule_languages(rule: &Rule, inferred_language: Option<&str>) -> Vec<String> {
    if !rule.languages.is_empty() {
        return rule.languages.clone();
    }

    match inferred_language {
        Some("shared") | Some("all") => crate::types::all_supported_languages(),
        Some(language) => vec![language.to_string()],
        None => Vec::new(),
    }
}

fn target_languages_for_filter(languages: &[String], lang_filter: Option<&str>) -> Vec<String> {
    match lang_filter {
        Some(filter) => languages
            .iter()
            .filter(|language| crate::types::language_matches_language(language, filter))
            .cloned()
            .collect(),
        None => languages.to_vec(),
    }
}

#[derive(Clone)]
pub struct ApprovedRule {
    pub id: String,
    pub severity: String,
    pub category: String,
    pub description: String,
    pub source_url: String,
    pub source_name: String,
    pub language: String,
    pub signals: Vec<ApprovedSignal>,
    pub formatter: Option<ApprovedFormatterDirective>,
    pub tests: Vec<ApprovedTestBinding>,
    pub validators: Vec<ApprovedValidatorBinding>,
    pub golden_examples: Vec<ApprovedExample>,
}

#[derive(Debug, Clone)]
pub struct ApprovedFormatterDirective {
    pub tool: String,
    pub options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ApprovedLintBinding {
    pub tool: String,
    pub code: String,
}

#[derive(Debug, Clone)]
pub struct ApprovedTestBinding {
    pub runner: String,
    pub path: String,
    pub selector: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApprovedValidatorBinding {
    pub adapter: String,
    pub rule: String,
    pub config: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone)]
pub struct ApprovedSignal {
    pub id: String,
    pub strategy: String,
    pub description: String,
    pub ast_query: Option<String>,
    pub lint: Option<ApprovedLintBinding>,
}

#[derive(Clone)]
pub struct ApprovedExample {
    pub code: String,
    pub verdict: String,
    pub language: Option<String>,
}

pub fn parse_legacy_lint_bindings(description: &str) -> Vec<ApprovedLintBinding> {
    let mut out = Vec::new();
    let parts: Vec<&str> = description.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        let normalized = part.to_ascii_lowercase();
        if VALID_LINT_TOOLS.contains(&normalized.as_str()) && i + 1 < parts.len() {
            out.push(ApprovedLintBinding {
                tool: normalized,
                code: parts[i + 1].trim_matches(&[',', '.', ';'][..]).to_string(),
            });
        }
    }
    out
}

pub fn approved_signal_lint_bindings(signal: &ApprovedSignal) -> Vec<ApprovedLintBinding> {
    signal
        .lint
        .clone()
        .map(|binding| vec![binding])
        .unwrap_or_else(|| parse_legacy_lint_bindings(&signal.description))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> RuleFile {
        serde_yaml::from_str(yaml).expect("valid rule yaml")
    }

    fn rule_file(signals: &str) -> RuleFile {
        parse(&format!(
            "source:\n  name: demo\nrules:\n  - id: demo.r\n    severity: should\n    confidence: high\n    category: convention\n    description: x\n    source_url: https://example.com\n    signals:\n{signals}"
        ))
    }

    #[test]
    fn pattern_signal_is_flagged_non_executable() {
        let rf = rule_file(
            "      - id: s\n        strategy: pattern\n        weight: required\n        match: 'foo'\n",
        );
        let warnings = validate_rule_file(&rf, "demo.yaml");
        assert!(warnings.iter().any(|w| w
            .message
            .contains("non-executable legacy `strategy: pattern`")));
    }

    #[test]
    fn ast_scope_does_not_make_legacy_pattern_executable() {
        let rf = rule_file(
            "      - id: s\n        strategy: pattern\n        weight: required\n        match: 'foo'\n        ast_scope: function_definition\n",
        );
        let warnings = validate_rule_file(&rf, "demo.yaml");
        assert!(warnings.iter().any(|w| w
            .message
            .contains("non-executable legacy `strategy: pattern`")));
    }

    #[test]
    fn ast_signal_without_query_is_flagged() {
        let rf = rule_file("      - id: s\n        strategy: ast\n        weight: required\n");
        let warnings = validate_rule_file(&rf, "demo.yaml");
        assert!(warnings
            .iter()
            .any(|w| w.message.contains("has no ast_query")));
    }

    #[test]
    fn ast_signal_with_query_is_clean() {
        let rf = rule_file(
            "      - id: s\n        strategy: ast\n        weight: required\n        ast_query: '(function_definition) @match'\n",
        );
        let warnings = validate_rule_file(&rf, "demo.yaml");
        assert!(!warnings
            .iter()
            .any(|w| w.message.contains("strategy: pattern")));
        assert!(!warnings.iter().any(|w| w.message.contains("ast_query")));
    }
}
