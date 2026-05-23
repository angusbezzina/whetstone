//! Verify `lint_proxy` signals against the project's linter config.
//!
//! A `lint_proxy` signal declares that an existing linter rule covers the
//! check (ruff E501, biome `suspicious/noExplicitAny`, etc.). `wh tests`
//! produces overlay configs that turn those rules on; this module walks
//! the project's primary linter config and reports any mapped rule that
//! is NOT enabled so the user knows enforcement is missing.
//!
//! Scope of linter support:
//! - **Ruff**: `ruff.toml`, `.ruff.toml`, `pyproject.toml` under
//!   `[tool.ruff.lint]` or `[tool.ruff]`, checking the `select =` list.
//! - **Biome**: `biome.json` / `biome.jsonc`, checking
//!   `linter.rules.<category>.<rule>` and the boolean `linter.enabled`.
//! - Clippy is deliberately skipped for now — its rule enablement is
//!   spread across `Cargo.toml` `[lints.clippy]`, `clippy.toml`, and
//!   in-source `#![warn(..)]` attributes, which is a larger investigation.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::{self, ApprovedRule, ApprovedValidatorBinding};

pub fn verify_lint_proxies(project_dir: &Path, rules: &[&ApprovedRule]) -> Vec<Value> {
    let ruff = load_ruff_selects(project_dir);
    let biome = load_biome_enabled(project_dir);
    let mut issues: Vec<Value> = Vec::new();

    for rule in rules {
        for sig in &rule.signals {
            if sig.strategy != "lint_proxy" {
                continue;
            }
            for binding in rules::approved_signal_lint_bindings(sig) {
                let verdict = match binding.tool.as_str() {
                    "ruff" => verify_ruff(&ruff, &binding.code),
                    "biome" => verify_biome(&biome, &binding.code),
                    _ => Verdict::Unsupported,
                };
                match verdict {
                    Verdict::Verified => continue,
                    Verdict::Missing => issues.push(json!({
                        "rule_id": rule.id,
                        "signal_id": sig.id,
                        "linter": binding.tool,
                        "code": binding.code,
                        "issue": "linter rule is not enabled in project config",
                        "fix": "run `wh tests` to generate the overlay config, or enable manually",
                        "config_files_checked": ruff.config_paths.iter().chain(biome.paths.iter()).map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    })),
                    Verdict::NoConfig => issues.push(json!({
                        "rule_id": rule.id,
                        "signal_id": sig.id,
                        "linter": binding.tool,
                        "code": binding.code,
                        "issue": "no linter config found to verify against",
                        "fix": "add ruff.toml / biome.json, or run `wh tests` for overlays",
                    })),
                    Verdict::InvalidConfig(err) => issues.push(json!({
                        "rule_id": rule.id,
                        "signal_id": sig.id,
                        "linter": binding.tool,
                        "code": binding.code,
                        "issue": format!("linter config could not be parsed: {err}"),
                        "fix": "fix the linter config syntax before relying on lint_proxy verification",
                    })),
                    Verdict::Unsupported => {
                        // Silently skip unsupported linters (e.g. clippy until
                        // we support it); treating them as issues would create
                        // noise.
                    }
                }
            }
        }
    }
    issues
}

pub fn verify_formatter_directives(project_dir: &Path, rules: &[&ApprovedRule]) -> Vec<Value> {
    let ruff = load_ruff_formatter(project_dir);
    let biome = load_biome_formatter(project_dir);
    let rustfmt = load_rustfmt_formatter(project_dir);
    let mut issues = Vec::new();

    for rule in rules {
        let Some(formatter) = &rule.formatter else {
            continue;
        };

        for (key, expected) in &formatter.options {
            let (paths, verdict) = match formatter.tool.as_str() {
                "ruff" => (
                    ruff.paths.clone(),
                    verify_formatter_value(&ruff.paths, ruff.options.get(key), expected),
                ),
                "biome" => (
                    biome.paths.clone(),
                    verify_formatter_value(&biome.paths, biome.options.get(key), expected),
                ),
                "rustfmt" => (
                    rustfmt.paths.clone(),
                    verify_formatter_value(&rustfmt.paths, rustfmt.options.get(key), expected),
                ),
                _ => (Vec::new(), Verdict::Unsupported),
            };

            match verdict {
                Verdict::Verified => {}
                Verdict::Missing => issues.push(json!({
                    "rule_id": rule.id,
                    "tool": formatter.tool,
                    "option": key,
                    "expected": expected,
                    "issue": "formatter option is not configured",
                    "fix": "run `wh actions lint` to generate the overlay config, or configure manually",
                    "config_files_checked": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                })),
                Verdict::NoConfig => issues.push(json!({
                    "rule_id": rule.id,
                    "tool": formatter.tool,
                    "option": key,
                    "expected": expected,
                    "issue": "no formatter config found to verify against",
                    "fix": "run `wh actions lint` to generate the overlay config, or configure manually",
                })),
                Verdict::InvalidConfig(err) => issues.push(json!({
                    "rule_id": rule.id,
                    "tool": formatter.tool,
                    "option": key,
                    "expected": expected,
                    "issue": format!("formatter config could not be parsed: {err}"),
                    "fix": "fix the formatter config syntax before relying on formatter verification",
                })),
                Verdict::Unsupported => {}
            }
        }
    }

    issues
}

pub fn verify_test_bindings(project_dir: &Path, rules: &[&ApprovedRule]) -> Vec<Value> {
    let mut issues = Vec::new();

    for rule in rules {
        for test in &rule.tests {
            let path = project_dir.join(&test.path);
            if !path.exists() {
                issues.push(json!({
                    "rule_id": rule.id,
                    "runner": test.runner,
                    "path": test.path,
                    "selector": test.selector,
                    "issue": "linked test path does not exist",
                    "fix": "create the referenced test file or update the rule binding",
                }));
                continue;
            }

            if let Some(selector) = &test.selector {
                match fs::read_to_string(&path) {
                    Ok(text) if text.contains(selector) => {}
                    Ok(_) => issues.push(json!({
                        "rule_id": rule.id,
                        "runner": test.runner,
                        "path": test.path,
                        "selector": selector,
                        "issue": "linked test selector was not found in the test file",
                        "fix": "update the selector or the referenced test file",
                    })),
                    Err(err) => issues.push(json!({
                        "rule_id": rule.id,
                        "runner": test.runner,
                        "path": test.path,
                        "selector": selector,
                        "issue": format!("failed to read linked test file: {err}"),
                        "fix": "fix the test file path or file permissions",
                    })),
                }
            }
        }
    }

    issues
}

pub fn verify_validator_bindings(project_dir: &Path, rules: &[&ApprovedRule]) -> Vec<Value> {
    let ruff = load_ruff_selects(project_dir);
    let biome = load_biome_enabled(project_dir);
    let mut issues = Vec::new();

    for rule in rules {
        for validator in &rule.validators {
            match validator.adapter.as_str() {
                "lint_rule" => {
                    let Some(tool) = validator_config_str(validator, "tool") else {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "issue": "validator config is missing `tool`",
                            "fix": "set validators[].config.tool to a supported linter name",
                        }));
                        continue;
                    };
                    let Some(code) = validator_config_str(validator, "code") else {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "issue": "validator config is missing `code`",
                            "fix": "set validators[].config.code to the linter rule code",
                        }));
                        continue;
                    };

                    let verdict = match tool {
                        "ruff" => verify_ruff(&ruff, code),
                        "biome" => verify_biome(&biome, code),
                        _ => Verdict::Unsupported,
                    };

                    match verdict {
                        Verdict::Verified => {}
                        Verdict::Missing => issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "linter": tool,
                            "code": code,
                            "issue": "validator-backed linter rule is not enabled in project config",
                            "fix": "enable the rule in project config or switch to a different validator adapter",
                        })),
                        Verdict::NoConfig => issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "linter": tool,
                            "code": code,
                            "issue": "no linter config found to verify validator binding against",
                            "fix": "add the relevant linter config or use a different validator adapter",
                        })),
                        Verdict::InvalidConfig(err) => issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "linter": tool,
                            "code": code,
                            "issue": format!("linter config could not be parsed: {err}"),
                            "fix": "fix the linter config syntax before relying on validator-backed lint verification",
                        })),
                        Verdict::Unsupported => issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "linter": tool,
                            "code": code,
                            "issue": "validator-backed linter tool is unsupported by Whetstone",
                            "fix": "use a supported tool or the `command` adapter for custom validation",
                        })),
                    }
                }
                "linked_test" => {
                    let Some(path) = validator_config_str(validator, "path") else {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "issue": "validator config is missing `path`",
                            "fix": "set validators[].config.path to a repo-relative test file",
                        }));
                        continue;
                    };
                    let runner = validator_config_str(validator, "runner").unwrap_or("unknown");
                    let selector = validator_config_str(validator, "selector");
                    if !is_safe_repo_relative(path) {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "path": path,
                            "issue": "linked_test validator path must stay within the repo",
                            "fix": "use a repo-relative test path without absolute or parent-directory traversal",
                        }));
                        continue;
                    }
                    let full = project_dir.join(path);
                    if !full.exists() {
                        issues.extend(verify_linked_test_path(
                            project_dir,
                            &rule.id,
                            runner,
                            path,
                            selector,
                            Some((&validator.adapter, &validator.rule)),
                        ));
                        continue;
                    }
                    if !path_resolves_within_repo(project_dir, &full) {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "path": path,
                            "issue": "linked_test validator path resolves outside the repo",
                            "fix": "remove symlink/path indirection that escapes the repository",
                        }));
                        continue;
                    }
                    issues.extend(verify_linked_test_path(
                        project_dir,
                        &rule.id,
                        runner,
                        path,
                        selector,
                        Some((&validator.adapter, &validator.rule)),
                    ));
                }
                "command" => {
                    let path = validator_config_str(validator, "path");
                    let command = validator_config_str(validator, "command");
                    if path.is_none() && command.is_none() {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "issue": "command validator requires either config.path or config.command",
                            "fix": "set validators[].config.path to a repo-relative executable or config.command to a shell command",
                        }));
                        continue;
                    }
                    if command.is_some()
                        && validator
                            .config
                            .get("allow_shell")
                            .and_then(|value| value.as_bool())
                            != Some(true)
                    {
                        issues.push(json!({
                            "rule_id": rule.id,
                            "adapter": validator.adapter,
                            "validator_rule": validator.rule,
                            "issue": "command validator using config.command requires config.allow_shell=true",
                            "fix": "set config.allow_shell=true or prefer config.path for repo-local executables",
                        }));
                    }
                    if command.is_none() {
                        if let Some(path) = path {
                            if !is_safe_repo_relative(path) {
                                issues.push(json!({
                                    "rule_id": rule.id,
                                    "adapter": validator.adapter,
                                    "validator_rule": validator.rule,
                                    "path": path,
                                    "issue": "command validator path must stay within the repo",
                                    "fix": "use a repo-relative validator path without absolute or parent-directory traversal",
                                }));
                                continue;
                            }
                            let full = project_dir.join(path);
                            if !full.exists() {
                                issues.push(json!({
                                "rule_id": rule.id,
                                "adapter": validator.adapter,
                                "validator_rule": validator.rule,
                                "path": path,
                                "issue": "command validator path does not exist",
                                "fix": "create the referenced validator script or update validators[].config.path",
                            }));
                                continue;
                            }
                            if !path_resolves_within_repo(project_dir, &full) {
                                issues.push(json!({
                                "rule_id": rule.id,
                                "adapter": validator.adapter,
                                "validator_rule": validator.rule,
                                    "path": path,
                                    "issue": "command validator path resolves outside the repo",
                                "fix": "remove symlink/path indirection that escapes the repository",
                            }));
                                continue;
                            }
                            if !path_is_executable(&full) {
                                issues.push(json!({
                                "rule_id": rule.id,
                                "adapter": validator.adapter,
                                    "validator_rule": validator.rule,
                                    "path": path,
                                    "issue": "command validator path is not executable",
                                    "fix": "mark the validator script executable or use config.command with allow_shell=true",
                                }));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    issues
}

enum Verdict {
    Verified,
    Missing,
    NoConfig,
    InvalidConfig(String),
    Unsupported,
}

fn validator_config_str<'a>(validator: &'a ApprovedValidatorBinding, key: &str) -> Option<&'a str> {
    validator.config.get(key).and_then(|value| value.as_str())
}

fn is_safe_repo_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn path_resolves_within_repo(project_dir: &Path, full_path: &Path) -> bool {
    let Ok(repo_root) = std::fs::canonicalize(project_dir) else {
        return false;
    };
    let Ok(resolved) = std::fs::canonicalize(full_path) else {
        return false;
    };
    resolved.starts_with(repo_root)
}

fn path_is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn verify_linked_test_path(
    project_dir: &Path,
    rule_id: &str,
    runner: &str,
    path: &str,
    selector: Option<&str>,
    validator_context: Option<(&str, &str)>,
) -> Vec<Value> {
    let mut issues = Vec::new();
    let full_path = project_dir.join(path);
    if !full_path.exists() {
        let mut issue = json!({
            "rule_id": rule_id,
            "runner": runner,
            "path": path,
            "selector": selector,
            "issue": "linked test path does not exist",
            "fix": "create the referenced test file or update the rule binding",
        });
        if let Some((adapter, validator_rule)) = validator_context {
            issue["adapter"] = Value::String(adapter.to_string());
            issue["validator_rule"] = Value::String(validator_rule.to_string());
        }
        issues.push(issue);
        return issues;
    }

    if let Some(selector) = selector {
        match fs::read_to_string(&full_path) {
            Ok(text) if text.contains(selector) => {}
            Ok(_) => {
                let mut issue = json!({
                    "rule_id": rule_id,
                    "runner": runner,
                    "path": path,
                    "selector": selector,
                    "issue": "linked test selector was not found in the test file",
                    "fix": "update the selector or the referenced test file",
                });
                if let Some((adapter, validator_rule)) = validator_context {
                    issue["adapter"] = Value::String(adapter.to_string());
                    issue["validator_rule"] = Value::String(validator_rule.to_string());
                }
                issues.push(issue);
            }
            Err(err) => {
                let mut issue = json!({
                    "rule_id": rule_id,
                    "runner": runner,
                    "path": path,
                    "selector": selector,
                    "issue": format!("failed to read linked test file: {err}"),
                    "fix": "fix the test file path or file permissions",
                });
                if let Some((adapter, validator_rule)) = validator_context {
                    issue["adapter"] = Value::String(adapter.to_string());
                    issue["validator_rule"] = Value::String(validator_rule.to_string());
                }
                issues.push(issue);
            }
        }
    }

    issues
}

// ── Ruff ──

struct FormatterConfig {
    paths: Vec<PathBuf>,
    options: std::collections::BTreeMap<String, Value>,
}

struct RuffConfig {
    config_paths: Vec<PathBuf>,
    selects: Vec<String>,
    ignores: Vec<String>,
    parse_errors: Vec<String>,
}

impl RuffConfig {
    fn has_code(&self, code: &str) -> bool {
        if self
            .ignores
            .iter()
            .any(|ignore| code_matches_ruff_select(ignore, code))
        {
            return false;
        }
        if self.selects.iter().any(|s| s.eq_ignore_ascii_case("ALL")) {
            return true;
        }
        self.selects
            .iter()
            .any(|s| code_matches_ruff_select(s, code))
    }
}

fn load_ruff_selects(project_dir: &Path) -> RuffConfig {
    let candidates = [
        project_dir.join("ruff.toml"),
        project_dir.join(".ruff.toml"),
        project_dir.join("pyproject.toml"),
        project_dir
            .join("whetstone")
            .join("lint")
            .join("ruff.whetstone.toml"),
        project_dir
            .join("whetstone")
            .join(".personal")
            .join("lint")
            .join("ruff.whetstone.toml"),
    ];
    let mut selects: Vec<String> = Vec::new();
    let mut ignores: Vec<String> = Vec::new();
    let mut config_paths: Vec<PathBuf> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(path) {
            let parsed: toml::Value = match toml::from_str(&text) {
                Ok(v) => v,
                Err(err) => {
                    parse_errors.push(format!("{}: {err}", path.display()));
                    continue;
                }
            };
            config_paths.push(path.clone());
            let root = if path
                .file_name()
                .map(|f| f == "pyproject.toml")
                .unwrap_or(false)
            {
                parsed
                    .get("tool")
                    .and_then(|t| t.get("ruff"))
                    .cloned()
                    .unwrap_or_else(|| toml::Value::Table(Default::default()))
            } else {
                parsed
            };
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("select"))
                .or_else(|| root.get("select"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        selects.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("extend-select"))
                .or_else(|| root.get("extend-select"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        selects.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("ignore"))
                .or_else(|| root.get("ignore"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        ignores.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("extend-ignore"))
                .or_else(|| root.get("extend-ignore"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        ignores.push(s.to_string());
                    }
                }
            }
        }
    }
    RuffConfig {
        config_paths,
        selects,
        ignores,
        parse_errors,
    }
}

/// Ruff `select` entries are either exact codes (`E501`) or prefixes that
/// match a family (`E`, `B`, `B006`). A rule's code matches if any select
/// entry is a prefix of it.
fn code_matches_ruff_select(select: &str, code: &str) -> bool {
    let s = select.trim();
    code.eq_ignore_ascii_case(s)
        || code
            .to_ascii_uppercase()
            .starts_with(&s.to_ascii_uppercase())
}

fn verify_ruff(cfg: &RuffConfig, code: &str) -> Verdict {
    if !cfg.parse_errors.is_empty() {
        return Verdict::InvalidConfig(cfg.parse_errors.join("; "));
    }
    if cfg.config_paths.is_empty() {
        return Verdict::NoConfig;
    }
    if cfg.has_code(code) {
        Verdict::Verified
    } else {
        Verdict::Missing
    }
}

fn load_ruff_formatter(project_dir: &Path) -> FormatterConfig {
    let candidates = [
        project_dir.join("ruff.toml"),
        project_dir.join(".ruff.toml"),
        project_dir.join("pyproject.toml"),
        project_dir
            .join("whetstone")
            .join("lint")
            .join("ruff.whetstone.toml"),
        project_dir
            .join("whetstone")
            .join(".personal")
            .join("lint")
            .join("ruff.whetstone.toml"),
    ];
    let mut paths = Vec::new();
    let mut options = std::collections::BTreeMap::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        paths.push(path.clone());
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(parsed) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        let root = if path
            .file_name()
            .map(|f| f == "pyproject.toml")
            .unwrap_or(false)
        {
            parsed
                .get("tool")
                .and_then(|t| t.get("ruff"))
                .cloned()
                .unwrap_or_else(|| toml::Value::Table(Default::default()))
        } else {
            parsed
        };
        if let Some(formatter) = root.get("format") {
            merge_toml_table_into_json_map(formatter, &mut options);
        }
    }
    FormatterConfig { paths, options }
}

// ── Biome ──

struct BiomeConfig {
    paths: Vec<PathBuf>,
    enabled: std::collections::BTreeSet<String>,
    parse_errors: Vec<String>,
}

fn load_biome_enabled(project_dir: &Path) -> BiomeConfig {
    let candidates = [
        project_dir.join("biome.json"),
        project_dir.join("biome.jsonc"),
        project_dir
            .join("whetstone")
            .join("lint")
            .join("biome.whetstone.json"),
        project_dir
            .join("whetstone")
            .join(".personal")
            .join("lint")
            .join("biome.whetstone.json"),
    ];
    let mut paths = Vec::new();
    let mut enabled = std::collections::BTreeSet::new();
    let mut parse_errors = Vec::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        // Strip JSONC comments defensively — biome.jsonc is allowed.
        let cleaned = strip_jsonc_comments(&text);
        let parsed: serde_json::Value = match serde_json::from_str(&cleaned) {
            Ok(v) => v,
            Err(err) => {
                parse_errors.push(format!("{}: {err}", path.display()));
                continue;
            }
        };
        paths.push(path.clone());
        let linter = parsed.get("linter");
        if linter
            .and_then(|l| l.get("enabled"))
            .and_then(|v| v.as_bool())
            == Some(false)
        {
            continue;
        }
        if let Some(rules) = linter
            .and_then(|l| l.get("rules"))
            .and_then(|r| r.as_object())
        {
            for (category, body) in rules {
                if let Some(obj) = body.as_object() {
                    for (name, severity) in obj {
                        let active = match severity {
                            serde_json::Value::String(s) => {
                                matches!(s.as_str(), "error" | "warn" | "info")
                            }
                            serde_json::Value::Object(o) => o
                                .get("level")
                                .and_then(|v| v.as_str())
                                .map(|s| matches!(s, "error" | "warn" | "info"))
                                .unwrap_or(false),
                            _ => false,
                        };
                        if active {
                            enabled.insert(format!("{category}/{name}"));
                        }
                    }
                }
            }
        }
    }
    BiomeConfig {
        paths,
        enabled,
        parse_errors,
    }
}

fn verify_biome(cfg: &BiomeConfig, code: &str) -> Verdict {
    if !cfg.parse_errors.is_empty() {
        return Verdict::InvalidConfig(cfg.parse_errors.join("; "));
    }
    if cfg.paths.is_empty() {
        return Verdict::NoConfig;
    }
    if cfg.enabled.contains(code) {
        Verdict::Verified
    } else {
        Verdict::Missing
    }
}

fn load_biome_formatter(project_dir: &Path) -> FormatterConfig {
    let candidates = [
        project_dir.join("biome.json"),
        project_dir.join("biome.jsonc"),
        project_dir
            .join("whetstone")
            .join("lint")
            .join("biome.whetstone.json"),
        project_dir
            .join("whetstone")
            .join(".personal")
            .join("lint")
            .join("biome.whetstone.json"),
    ];
    let mut paths = Vec::new();
    let mut options = std::collections::BTreeMap::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        paths.push(path.clone());
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let cleaned = strip_jsonc_comments(&text);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&cleaned) else {
            continue;
        };
        if let Some(formatter) = parsed.get("formatter").and_then(|v| v.as_object()) {
            for (key, value) in formatter {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    FormatterConfig { paths, options }
}

fn load_rustfmt_formatter(project_dir: &Path) -> FormatterConfig {
    let candidates = [
        project_dir.join("rustfmt.toml"),
        project_dir.join(".rustfmt.toml"),
        project_dir
            .join("whetstone")
            .join("lint")
            .join("rustfmt.whetstone.toml"),
        project_dir
            .join("whetstone")
            .join(".personal")
            .join("lint")
            .join("rustfmt.whetstone.toml"),
    ];
    let mut paths = Vec::new();
    let mut options = std::collections::BTreeMap::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        paths.push(path.clone());
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(parsed) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        merge_toml_table_into_json_map(&parsed, &mut options);
    }
    FormatterConfig { paths, options }
}

fn merge_toml_table_into_json_map(
    value: &toml::Value,
    out: &mut std::collections::BTreeMap<String, Value>,
) {
    if let Some(table) = value.as_table() {
        for (key, value) in table {
            if let Some(json) = toml_to_json(value) {
                out.insert(key.clone(), json);
            }
        }
    }
}

fn toml_to_json(value: &toml::Value) -> Option<Value> {
    match value {
        toml::Value::String(v) => Some(Value::String(v.clone())),
        toml::Value::Integer(v) => Some(json!(v)),
        toml::Value::Float(v) => Some(json!(v)),
        toml::Value::Boolean(v) => Some(Value::Bool(*v)),
        _ => None,
    }
}

fn verify_formatter_value(paths: &[PathBuf], actual: Option<&Value>, expected: &Value) -> Verdict {
    if paths.is_empty() {
        return Verdict::NoConfig;
    }
    match actual {
        Some(actual) if actual == expected => Verdict::Verified,
        _ => Verdict::Missing,
    }
}

/// Minimal JSONC → JSON comment stripper: removes `//` line comments and
/// `/* */` block comments while preserving strings. Good enough for biome
/// configs, not a general-purpose JSONC parser.
fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            out.push(c as char);
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'/' => {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'*' => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                _ => {}
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn validator_rule(
        binding: crate::rules::ApprovedValidatorBinding,
    ) -> crate::rules::ApprovedRule {
        crate::rules::ApprovedRule {
            id: "demo.validator".into(),
            severity: "should".into(),
            confidence: "high".into(),
            category: "convention".into(),
            description: "desc".into(),
            source_url: "https://example.com".into(),
            source_name: "demo".into(),
            language: "javascript".into(),
            languages: vec!["javascript".into()],
            signals: Vec::new(),
            formatter: None,
            tests: Vec::new(),
            validators: vec![binding],
            provenance: None,
            golden_examples: Vec::new(),
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
        }
    }

    #[test]
    fn ruff_prefix_select_matches_subcode() {
        assert!(code_matches_ruff_select("E", "E501"));
        assert!(code_matches_ruff_select("B006", "B006"));
        assert!(!code_matches_ruff_select("F", "E501"));
    }

    #[test]
    fn legacy_lint_binding_parser_recognizes_ruff_and_biome() {
        let got = rules::parse_legacy_lint_bindings(
            "Covered by ruff B006 and biome suspicious/noExplicitAny.",
        );
        assert!(got
            .iter()
            .any(|binding| binding.tool == "ruff" && binding.code == "B006"));
        assert!(got.iter().any(|binding| {
            binding.tool == "biome" && binding.code == "suspicious/noExplicitAny"
        }));
    }

    #[test]
    fn strip_jsonc_removes_comments_outside_strings() {
        let src = r#"{
            // trailing line
            "k": "has // inner",
            /* block */
            "v": 1
        }"#;
        let cleaned = strip_jsonc_comments(src);
        assert!(!cleaned.contains("trailing line"));
        assert!(cleaned.contains("has // inner"));
    }

    #[test]
    fn formatter_value_verification_uses_exact_json_match() {
        let paths = vec![PathBuf::from("ruff.toml")];
        assert!(matches!(
            verify_formatter_value(&paths, Some(&json!("single")), &json!("single")),
            Verdict::Verified
        ));
        assert!(matches!(
            verify_formatter_value(&paths, Some(&json!("double")), &json!("single")),
            Verdict::Missing
        ));
    }

    #[test]
    fn validator_command_requires_path_or_command() {
        let tmp = tempfile::tempdir().unwrap();
        let rule = validator_rule(crate::rules::ApprovedValidatorBinding {
            adapter: "command".into(),
            rule: "custom.inline-handlers".into(),
            mode: None,
            config: Default::default(),
        });

        let issues = verify_validator_bindings(tmp.path(), &[&rule]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["adapter"], "command");
    }

    #[test]
    fn validator_linked_test_checks_path_presence() {
        let tmp = tempfile::tempdir().unwrap();
        let rule = validator_rule(crate::rules::ApprovedValidatorBinding {
            adapter: "linked_test".into(),
            rule: "custom.inline-handlers".into(),
            mode: None,
            config: BTreeMap::from([
                ("runner".into(), json!("vitest")),
                ("path".into(), json!("tests/inline-handlers.test.ts")),
            ]),
        });

        let issues = verify_validator_bindings(tmp.path(), &[&rule]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["adapter"], "linked_test");
        assert_eq!(issues[0]["path"], "tests/inline-handlers.test.ts");
    }
}
