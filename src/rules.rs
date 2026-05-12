//! Structured YAML rule loading and validation.
//!
//! Replaces the regex-based rule parsing with serde_yaml deserialization
//! and provides validation against the rule schema.

use serde::{Deserialize, Serialize};
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
const VALID_STRATEGIES: &[&str] = &["ast", "pattern", "lint_proxy"];

/// Valid severity levels.
const VALID_SEVERITIES: &[&str] = &["must", "should", "may"];

/// Valid confidence levels.
const VALID_CONFIDENCES: &[&str] = &["high", "medium"];

/// Valid rule lifecycle statuses.
/// See `references/handoff-schema.md` and `references/workflow-matrix.md`.
const VALID_STATUSES: &[&str] = &["candidate", "approved"];

/// Supported lint tools for structured lint bindings.
const VALID_LINT_TOOLS: &[&str] = &["ruff", "biome", "clippy"];

/// Supported formatter tools for formatter-backed custom rules.
const VALID_FORMATTER_TOOLS: &[&str] = &["ruff", "biome", "rustfmt"];

/// Supported explicit test runners for test-backed custom rules.
const VALID_TEST_RUNNERS: &[&str] = &["pytest", "vitest", "cargo"];

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
    /// Concrete regex pattern for `pattern` strategy signals.
    /// When present, test generation produces real regex checks instead of TODO stubs.
    #[serde(default, alias = "match")]
    pub match_pattern: Option<String>,
    /// Raw tree-sitter query (S-expression) for `ast` signals. Every node
    /// captured by `@match` is reported as a violation. When present, the
    /// `wh check` runner uses the project's tree-sitter grammar for the
    /// rule's language; when absent, an `ast` signal falls back to regex
    /// scanning of the file text.
    #[serde(default)]
    pub ast_query: Option<String>,
    /// AST node kind that scopes a `pattern` signal's regex. When set, the
    /// regex is only applied inside nodes of that kind (e.g.
    /// `function_definition`), removing false positives from the
    /// surrounding file text like comments and module-level code.
    #[serde(default)]
    pub ast_scope: Option<String>,
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
            || !rule.tests.is_empty();
        if !has_enforcement_surface {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!(
                    "{rule_ctx}: no enforceable signal or binding (expected ast, pattern, lint_proxy, formatter, or tests)"
                ),
            });
        }

        if rule.signals.is_empty() && rule.formatter.is_none() && rule.tests.is_empty() {
            warnings.push(ValidationWarning {
                file: file_path.to_string(),
                message: format!("{rule_ctx}: no signals, formatter, or tests defined"),
            });
        }

        for sig in &rule.signals {
            if !VALID_STRATEGIES.contains(&sig.strategy.as_str()) {
                warnings.push(ValidationWarning {
                    file: file_path.to_string(),
                    message: format!("{rule_ctx}: signal has invalid strategy '{}'", sig.strategy),
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
        }
    }

    warnings
}

// --- Schema + fixtures validation (replacement for scripts/validate-rule-schema.py) ---

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

/// Fixture paths (relative to the project root) that are deliberately invalid
/// and should be skipped by the schema validator.
const INTENTIONALLY_INVALID_FIXTURES: &[&str] =
    &["tests/fixtures/whetstone/rules/python/malformed.yaml"];

/// The rule-schema YAML embedded at compile time so `wh validate` works for
/// downstream projects that do not carry the Whetstone source tree.
const EMBEDDED_SCHEMA: &str = include_str!("../references/rule-schema.yaml");

/// Validate the schema file and all rule fixtures under the given project root.
///
/// Produces a human-readable report (matching the legacy Python contract) and
/// a boolean indicating overall success. Used by the `validate-rules` CLI
/// subcommand and the CI schema gate.
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
        if INTENTIONALLY_INVALID_FIXTURES.iter().any(|p| rel == *p) {
            out.push_str(&format!("  SKIP: {rel} (intentional invalid fixture)\n"));
            continue;
        }

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
            if matches!(dir.as_str(), "python" | "typescript" | "rust" | "shared") {
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
        for rule in &lrf.rule_file.rules {
            if !rule.approved {
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
                    confidence: rule.confidence.clone().unwrap_or_default(),
                    category: rule.category.clone().unwrap_or_default(),
                    description: rule.description.clone().unwrap_or_default(),
                    source_url: rule.source_url.clone().unwrap_or_default(),
                    source_name: lrf.rule_file.source.name.clone(),
                    language,
                    languages: target_languages.clone(),
                    signals: rule
                        .signals
                        .iter()
                        .map(|s| ApprovedSignal {
                            id: s.id.clone().unwrap_or_default(),
                            strategy: s.strategy.clone(),
                            description: s.description.clone().unwrap_or_default(),
                            weight: s.weight.clone().unwrap_or_default(),
                            match_pattern: s.match_pattern.clone(),
                            ast_query: s.ast_query.clone(),
                            ast_scope: s.ast_scope.clone(),
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
                    deterministic_pass_threshold: rule.deterministic_pass_threshold,
                    deterministic_fail_threshold: rule.deterministic_fail_threshold,
                });
            }
        }
    }

    (approved, warnings)
}

/// Convert already-loaded rule files to approved rules (retained for
/// potential reuse; originally used by the built-in layer which was removed).
#[allow(dead_code)]
pub fn approved_from_loaded(
    loaded: &[LoadedRuleFile],
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let mut approved = Vec::new();

    for lrf in loaded {
        for rule in &lrf.rule_file.rules {
            if !rule.approved {
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
                    confidence: rule.confidence.clone().unwrap_or_default(),
                    category: rule.category.clone().unwrap_or_default(),
                    description: rule.description.clone().unwrap_or_default(),
                    source_url: rule.source_url.clone().unwrap_or_default(),
                    source_name: lrf.rule_file.source.name.clone(),
                    language,
                    languages: target_languages.clone(),
                    signals: rule
                        .signals
                        .iter()
                        .map(|s| ApprovedSignal {
                            id: s.id.clone().unwrap_or_default(),
                            strategy: s.strategy.clone(),
                            description: s.description.clone().unwrap_or_default(),
                            weight: s.weight.clone().unwrap_or_default(),
                            match_pattern: s.match_pattern.clone(),
                            ast_query: s.ast_query.clone(),
                            ast_scope: s.ast_scope.clone(),
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
                    deterministic_pass_threshold: rule.deterministic_pass_threshold,
                    deterministic_fail_threshold: rule.deterministic_fail_threshold,
                });
            }
        }
    }

    (approved, Vec::new())
}

fn resolved_rule_languages(rule: &Rule, inferred_language: Option<&str>) -> Vec<String> {
    if !rule.languages.is_empty() {
        return rule.languages.clone();
    }

    match inferred_language {
        Some("shared") | Some("all") => vec!["python".into(), "rust".into(), "typescript".into()],
        Some(language) => vec![language.to_string()],
        None => Vec::new(),
    }
}

fn target_languages_for_filter(languages: &[String], lang_filter: Option<&str>) -> Vec<String> {
    match lang_filter {
        Some(filter) => languages
            .iter()
            .filter(|language| language.as_str() == filter)
            .cloned()
            .collect(),
        None => languages.to_vec(),
    }
}

#[derive(Clone)]
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
    pub languages: Vec<String>,
    pub signals: Vec<ApprovedSignal>,
    pub formatter: Option<ApprovedFormatterDirective>,
    pub tests: Vec<ApprovedTestBinding>,
    pub golden_examples: Vec<ApprovedExample>,
    pub deterministic_pass_threshold: Option<u32>,
    pub deterministic_fail_threshold: Option<u32>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApprovedFormatterDirective {
    pub tool: String,
    pub options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApprovedLintBinding {
    pub tool: String,
    pub code: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApprovedTestBinding {
    pub runner: String,
    pub path: String,
    pub selector: Option<String>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct ApprovedSignal {
    pub id: String,
    pub strategy: String,
    pub description: String,
    pub weight: String,
    pub match_pattern: Option<String>,
    pub ast_query: Option<String>,
    pub ast_scope: Option<String>,
    pub lint: Option<ApprovedLintBinding>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct ApprovedExample {
    pub code: String,
    pub verdict: String,
    pub reason: String,
    pub language: Option<String>,
}

pub fn lint_tool_matches_language(tool: &str, language: &str) -> bool {
    matches!(
        (tool, language),
        ("ruff", "python") | ("biome", "typescript") | ("clippy", "rust")
    )
}

pub fn formatter_tool_matches_language(tool: &str, language: &str) -> bool {
    matches!(
        (tool, language),
        ("ruff", "python") | ("biome", "typescript") | ("rustfmt", "rust")
    )
}

pub fn test_runner_matches_language(runner: &str, language: &str) -> bool {
    matches!(
        (runner, language),
        ("pytest", "python") | ("vitest", "typescript") | ("cargo", "rust")
    )
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
