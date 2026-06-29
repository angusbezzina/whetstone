//! `wh rules add`, `wh rules edit`, and `wh rules remove` — direct rule authoring and mutation.
//!
//! Covers Epic 3E theme C (authoring shortcuts):
//! - `wh rules add` lets users write a personal preference in one command,
//!   skipping the extract/submit/approve dance. Rules land as
//!   `status: approved` directly (user is the author AND the approver).
//! - `wh rules edit` bumps severity / confidence on existing approved rules
//!   as taste matures. Bulk via `--all` + selectors.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::{self, load_rule_files};

// ── Shared validation sets (mirrored from rules.rs private constants) ──

const VALID_SEVERITIES: &[&str] = &["must", "should", "may"];
const VALID_CONFIDENCES: &[&str] = &["high", "medium"];
const VALID_CATEGORIES: &[&str] = &[
    "migration",
    "default",
    "convention",
    "breaking-change",
    "semantic",
];

// ── add ──

#[derive(Debug, Clone)]
pub enum EnforcementMode {
    Advisory,
    Pattern {
        regex: String,
        /// AST node kind that bounds the regex (the only sanctioned form of
        /// `strategy: pattern`). When None the signal is a bare pattern, which
        /// `wh validate` rejects for shipped project rules.
        ast_scope: Option<String>,
    },
    Lint {
        tool: String,
        code: String,
    },
    Formatter {
        tool: String,
        options: BTreeMap<String, Value>,
    },
    Test {
        runner: String,
        path: String,
        selector: Option<String>,
    },
    Validator {
        adapter: String,
        rule: String,
        config: BTreeMap<String, Value>,
    },
}

pub struct AddOptions {
    /// Full id (`dep.rule-name`) OR just `rule-name` with `dep` supplied.
    pub rule_id: String,
    pub description: String,
    pub severity: String,
    pub confidence: String,
    pub category: String,
    pub language: String,
    pub source_url: Option<String>,
    pub dep: Option<String>,
    pub enforcement: EnforcementMode,
    /// Target the personal layer (gitignored) rather than the committed project layer.
    pub personal: bool,
}

pub fn add(project_dir: &Path, opts: AddOptions) -> Result<Value> {
    validate_enum("severity", &opts.severity, VALID_SEVERITIES)?;
    validate_enum("confidence", &opts.confidence, VALID_CONFIDENCES)?;
    validate_enum("category", &opts.category, VALID_CATEGORIES)?;
    let normalized_language = normalize_rule_language(&opts.language)?;

    if opts.description.trim().is_empty() {
        return Err(anyhow!("--description is required and must be non-empty"));
    }

    validate_enforcement(&opts.enforcement, normalized_language)?;

    let (dep, full_id) = parse_id(&opts.rule_id, opts.dep.as_deref())?;
    let existing = collect_existing_rule_ids(project_dir);
    if existing.contains(&full_id) {
        return Err(anyhow!(
            "rule id `{full_id}` already exists in the project ruleset. Edit it with `wh rules edit` or pick a different id."
        ));
    }

    // Build the rule YAML mapping.
    let mut rule = Mapping::new();
    rule.insert(ystr("id"), ystr(&full_id));
    rule.insert(ystr("severity"), ystr(&opts.severity));
    rule.insert(ystr("confidence"), ystr(&opts.confidence));
    rule.insert(ystr("category"), ystr(&opts.category));
    rule.insert(ystr("description"), ystr(&opts.description));
    rule.insert(
        ystr("source_url"),
        ystr(
            opts.source_url
                .unwrap_or_else(|| format!("personal://{dep}/{full_id}"))
                .as_str(),
        ),
    );
    rule.insert(ystr("approved"), YamlValue::Bool(true));
    rule.insert(ystr("status"), ystr("approved"));
    if normalized_language == crate::types::ALL_LANGUAGE_META {
        rule.insert(
            ystr("languages"),
            YamlValue::Sequence(
                crate::types::all_supported_languages()
                    .into_iter()
                    .map(|language| ystr(&language))
                    .collect(),
            ),
        );
    }

    let mut signals = Vec::new();
    let mut tests = Vec::new();
    let mut validators = Vec::new();

    match &opts.enforcement {
        EnforcementMode::Advisory => {}
        EnforcementMode::Pattern { regex, ast_scope } => {
            let mut sig = Mapping::new();
            sig.insert(ystr("id"), ystr("authored-pattern"));
            sig.insert(ystr("strategy"), ystr("pattern"));
            sig.insert(ystr("description"), ystr("Authored regex"));
            sig.insert(ystr("weight"), ystr("required"));
            sig.insert(ystr("match"), ystr(regex));
            if let Some(scope) = ast_scope {
                sig.insert(ystr("ast_scope"), ystr(scope));
            }
            signals.push(YamlValue::Mapping(sig));
        }
        EnforcementMode::Lint { tool, code } => {
            let mut lint = Mapping::new();
            lint.insert(ystr("tool"), ystr(tool));
            lint.insert(ystr("code"), ystr(code));

            let mut sig = Mapping::new();
            sig.insert(ystr("id"), ystr("authored-lint"));
            sig.insert(ystr("strategy"), ystr("lint_proxy"));
            sig.insert(
                ystr("description"),
                ystr(&format!("Structured lint binding for {tool} {code}")),
            );
            sig.insert(ystr("weight"), ystr("required"));
            sig.insert(ystr("lint"), YamlValue::Mapping(lint));
            signals.push(YamlValue::Mapping(sig));
        }
        EnforcementMode::Formatter { tool, options } => {
            let mut formatter = Mapping::new();
            formatter.insert(ystr("tool"), ystr(tool));
            formatter.insert(ystr("options"), json_map_to_yaml(options));
            rule.insert(ystr("formatter"), YamlValue::Mapping(formatter));
        }
        EnforcementMode::Test {
            runner,
            path,
            selector,
        } => {
            let mut test = Mapping::new();
            test.insert(ystr("runner"), ystr(runner));
            test.insert(ystr("path"), ystr(path));
            if let Some(selector) = selector {
                test.insert(ystr("selector"), ystr(selector));
            }
            tests.push(YamlValue::Mapping(test));
        }
        EnforcementMode::Validator {
            adapter,
            rule,
            config,
        } => {
            let mut validator = Mapping::new();
            validator.insert(ystr("adapter"), ystr(adapter));
            validator.insert(ystr("rule"), ystr(rule));
            if !config.is_empty() {
                validator.insert(ystr("config"), json_map_to_yaml(config));
            }
            validators.push(YamlValue::Mapping(validator));
        }
    }
    rule.insert(ystr("signals"), YamlValue::Sequence(signals));
    if !tests.is_empty() {
        rule.insert(ystr("tests"), YamlValue::Sequence(tests));
    }
    if !validators.is_empty() {
        rule.insert(ystr("validators"), YamlValue::Sequence(validators));
    }

    // Two golden examples: a pass+fail. The user can edit them later; this keeps
    // `wh validate` happy (it requires at least one example).
    let mut pass_ex = Mapping::new();
    pass_ex.insert(
        ystr("code"),
        ystr("// TODO: a code snippet that PASSES this rule"),
    );
    pass_ex.insert(ystr("verdict"), ystr("pass"));
    pass_ex.insert(ystr("reason"), ystr("Adheres to the authored rule"));
    let mut fail_ex = Mapping::new();
    fail_ex.insert(
        ystr("code"),
        ystr("// TODO: a code snippet that FAILS this rule"),
    );
    fail_ex.insert(ystr("verdict"), ystr("fail"));
    fail_ex.insert(ystr("reason"), ystr("Violates the authored rule"));
    rule.insert(
        ystr("golden_examples"),
        YamlValue::Sequence(vec![
            YamlValue::Mapping(pass_ex),
            YamlValue::Mapping(fail_ex),
        ]),
    );

    // Append to the existing dep file when present, otherwise create it.
    let dest = destination_path(project_dir, opts.personal, normalized_language, &dep);
    let dest_existed = dest.exists();

    let mut top = if dest_existed {
        read_yaml_mapping(&dest)?
    } else {
        let mut m = Mapping::new();
        let mut src = Mapping::new();
        src.insert(ystr("name"), ystr(&dep));
        m.insert(ystr("source"), YamlValue::Mapping(src));
        m.insert(ystr("rules"), YamlValue::Sequence(Vec::new()));
        m
    };

    let rules_slot = top
        .entry(ystr("rules"))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()));
    let rules_seq = match rules_slot {
        YamlValue::Sequence(seq) => seq,
        _ => return Err(anyhow!("{} has a non-sequence `rules` key", dest.display())),
    };
    rules_seq.push(YamlValue::Mapping(rule));

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_yaml::to_string(&YamlValue::Mapping(top))?;
    fs::write(&dest, body)?;

    Ok(json!({
        "status": "ok",
        "wrote": dest.display().to_string(),
        "created_file": !dest_existed,
        "rule_id": full_id,
        "dependency": dep,
        "layer": if opts.personal { "personal" } else { "project" },
        "next_command": "wh actions all",
    }))
}

fn validate_enforcement(enforcement: &EnforcementMode, language: &str) -> Result<()> {
    match enforcement {
        EnforcementMode::Advisory => Ok(()),
        EnforcementMode::Pattern { regex, .. } => {
            if regex.trim().is_empty() {
                Err(anyhow!("pattern enforcement requires a non-empty regex"))
            } else {
                Ok(())
            }
        }
        EnforcementMode::Lint { tool, code } => {
            if tool.trim().is_empty() || code.trim().is_empty() {
                return Err(anyhow!("lint enforcement requires non-empty tool and code"));
            }
            if !rules::lint_tool_matches_language(tool, language) {
                return Err(anyhow!(
                    "lint tool `{tool}` is not supported for language `{language}`"
                ));
            }
            Ok(())
        }
        EnforcementMode::Formatter { tool, options } => {
            if tool.trim().is_empty() {
                return Err(anyhow!("formatter enforcement requires a formatter tool"));
            }
            if options.is_empty() {
                return Err(anyhow!(
                    "formatter enforcement requires at least one formatter option"
                ));
            }
            if !rules::formatter_tool_matches_language(tool, language) {
                return Err(anyhow!(
                    "formatter tool `{tool}` is not supported for language `{language}`"
                ));
            }
            Ok(())
        }
        EnforcementMode::Test {
            runner,
            path,
            selector: _,
        } => {
            if runner.trim().is_empty() || path.trim().is_empty() {
                return Err(anyhow!("test enforcement requires a runner and path"));
            }
            if !rules::test_runner_matches_language(runner, language) {
                return Err(anyhow!(
                    "test runner `{runner}` is not supported for language `{language}`"
                ));
            }
            Ok(())
        }
        EnforcementMode::Validator {
            adapter,
            rule,
            config: _,
        } => {
            if adapter.trim().is_empty() || rule.trim().is_empty() {
                return Err(anyhow!(
                    "validator enforcement requires non-empty adapter and rule"
                ));
            }
            Ok(())
        }
    }
}

// ── edit ──

pub struct EditSelector<'a> {
    pub rule_id: Option<&'a str>,
    pub all: bool,
    pub dep: Option<&'a str>,
    pub category: Option<&'a str>,
}

pub struct EditMutation<'a> {
    pub severity: Option<&'a str>,
    pub confidence: Option<&'a str>,
}

pub struct RemoveOptions<'a> {
    pub rule_id: &'a str,
}

pub fn edit(
    project_dir: &Path,
    selector: EditSelector<'_>,
    mutation: EditMutation<'_>,
    dry_run: bool,
) -> Result<Value> {
    if mutation.severity.is_none() && mutation.confidence.is_none() {
        return Err(anyhow!(
            "nothing to change. Pass at least one of --severity or --confidence"
        ));
    }
    if let Some(sev) = mutation.severity {
        validate_enum("severity", sev, VALID_SEVERITIES)?;
    }
    if let Some(conf) = mutation.confidence {
        validate_enum("confidence", conf, VALID_CONFIDENCES)?;
    }

    let single_target = match (selector.rule_id, selector.all) {
        (Some(id), false) => Some(id),
        (None, true) => None,
        (Some(_), true) => {
            return Err(anyhow!("pass either <rule-id> or --all, not both"));
        }
        (None, false) => {
            return Err(anyhow!(
                "must specify a <rule-id> argument or --all with selectors"
            ));
        }
    };

    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let mut edits: Vec<EditRecord> = Vec::new();
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = load_rule_files(dir);
        for lrf in &files {
            let file_path = PathBuf::from(&lrf.file_path);
            let mut top = read_yaml_mapping(&file_path)?;
            let mut mutated = false;

            if let Some(YamlValue::Sequence(ref mut rules_seq)) = top.get_mut(ystr("rules")) {
                for rule in rules_seq.iter_mut() {
                    let YamlValue::Mapping(rule_map) = rule else {
                        continue;
                    };

                    let id = rule_map
                        .get(ystr("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();

                    if !matches_selector(rule_map, &id, single_target, &selector) {
                        continue;
                    }

                    let status = rule_map
                        .get(ystr("status"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("approved");
                    if status == "candidate" {
                        return Err(anyhow!(
                            "rule `{id}` is a candidate; approve it via `wh rules approve` before editing"
                        ));
                    }

                    let mut record = EditRecord {
                        rule_id: id.clone(),
                        file: file_path.display().to_string(),
                        before_severity: rule_map
                            .get(ystr("severity"))
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        after_severity: None,
                        before_confidence: rule_map
                            .get(ystr("confidence"))
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        after_confidence: None,
                    };

                    if let Some(new_sev) = mutation.severity {
                        rule_map.insert(ystr("severity"), ystr(new_sev));
                        record.after_severity = Some(new_sev.to_string());
                        mutated = true;
                    }
                    if let Some(new_conf) = mutation.confidence {
                        rule_map.insert(ystr("confidence"), ystr(new_conf));
                        record.after_confidence = Some(new_conf.to_string());
                        mutated = true;
                    }

                    edits.push(record);
                }
            }

            if mutated && !dry_run {
                let body = serde_yaml::to_string(&YamlValue::Mapping(top))?;
                fs::write(&file_path, body)?;
            }
        }
    }

    if edits.is_empty() {
        return Err(anyhow!(
            "no approved rules match the selector. Use `wh rules list` to inspect the ruleset."
        ));
    }

    Ok(json!({
        "status": "ok",
        "dry_run": dry_run,
        "changed": edits.iter().map(edit_record_to_json).collect::<Vec<_>>(),
        "count": edits.len(),
        "next_command": if dry_run { "wh rules edit <same args, without --dry-run>" } else { "wh actions all" },
    }))
}

// ── remove ──

pub fn remove(project_dir: &Path, opts: RemoveOptions<'_>) -> Result<Value> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut matches: Vec<(PathBuf, Mapping, usize)> = Vec::new();

    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = load_rule_files(dir);
        for lrf in &files {
            let file_path = PathBuf::from(&lrf.file_path);
            let top = read_yaml_mapping(&file_path)?;
            if let Some(YamlValue::Sequence(rules_seq)) = top.get(ystr("rules")) {
                for (idx, rule) in rules_seq.iter().enumerate() {
                    let YamlValue::Mapping(rule_map) = rule else {
                        continue;
                    };
                    let id = rule_map
                        .get(ystr("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if id == opts.rule_id {
                        matches.push((file_path.clone(), top.clone(), idx));
                    }
                }
            }
        }
    }

    if matches.is_empty() {
        return Err(anyhow!(
            "rule `{}` not found. Use `wh rules list` to inspect the ruleset.",
            opts.rule_id
        ));
    }
    if matches.len() > 1 {
        return Err(anyhow!(
            "rule `{}` appears in multiple files. Resolve the duplicate manually before using `wh rules remove`.",
            opts.rule_id
        ));
    }

    let (file_path, mut top, remove_idx) = matches.remove(0);
    let mut deleted_file = false;
    if let Some(YamlValue::Sequence(rules_seq)) = top.get_mut(ystr("rules")) {
        rules_seq.remove(remove_idx);
        if rules_seq.is_empty() {
            fs::remove_file(&file_path)
                .map_err(|e| anyhow!("failed to remove {}: {e}", file_path.display()))?;
            deleted_file = true;
        } else {
            let body = serde_yaml::to_string(&YamlValue::Mapping(top))?;
            fs::write(&file_path, body)
                .map_err(|e| anyhow!("failed to write {}: {e}", file_path.display()))?;
        }
    }

    Ok(json!({
        "status": "ok",
        "rule_id": opts.rule_id,
        "file": file_path.display().to_string(),
        "deleted_file": deleted_file,
        "next_command": "wh actions all",
    }))
}

struct EditRecord {
    rule_id: String,
    file: String,
    before_severity: Option<String>,
    after_severity: Option<String>,
    before_confidence: Option<String>,
    after_confidence: Option<String>,
}

fn edit_record_to_json(r: &EditRecord) -> Value {
    json!({
        "rule_id": r.rule_id,
        "file": r.file,
        "severity": { "before": r.before_severity, "after": r.after_severity },
        "confidence": { "before": r.before_confidence, "after": r.after_confidence },
    })
}

fn matches_selector(
    rule_map: &Mapping,
    rule_id: &str,
    single_target: Option<&str>,
    selector: &EditSelector<'_>,
) -> bool {
    if let Some(target) = single_target {
        return rule_id == target;
    }
    // --all mode: both dep and category filters are AND-combined.
    if let Some(dep) = selector.dep {
        let id_dep = rule_id.split('.').next().unwrap_or("");
        if id_dep != dep {
            return false;
        }
    }
    if let Some(category) = selector.category {
        let cat = rule_map
            .get(ystr("category"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if cat != category {
            return false;
        }
    }
    true
}

// ── helpers ──

fn json_map_to_yaml(map: &BTreeMap<String, Value>) -> YamlValue {
    let mut out = Mapping::new();
    for (key, value) in map {
        out.insert(ystr(key), json_value_to_yaml(value));
    }
    YamlValue::Mapping(out)
}

fn json_value_to_yaml(value: &Value) -> YamlValue {
    match value {
        Value::Null => YamlValue::Null,
        Value::Bool(v) => YamlValue::Bool(*v),
        Value::Number(v) => serde_yaml::to_value(v).unwrap_or(YamlValue::Null),
        Value::String(v) => ystr(v),
        Value::Array(values) => {
            YamlValue::Sequence(values.iter().map(json_value_to_yaml).collect())
        }
        Value::Object(obj) => {
            let mut out = Mapping::new();
            for (key, inner) in obj {
                out.insert(ystr(key), json_value_to_yaml(inner));
            }
            YamlValue::Mapping(out)
        }
    }
}

fn ystr(s: &str) -> YamlValue {
    YamlValue::String(s.to_string())
}

fn validate_enum(field: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid {field} `{value}`. Must be one of: {}",
            allowed.join(", ")
        ))
    }
}

fn parse_id(rule_id: &str, dep_override: Option<&str>) -> Result<(String, String)> {
    if let Some(dep) = dep_override {
        if dep.is_empty() {
            return Err(anyhow!("--dep cannot be empty"));
        }
        if rule_id.contains('.') {
            // User already qualified the id; verify the dep prefix matches.
            let first = rule_id.split('.').next().unwrap_or_default();
            if first != dep {
                return Err(anyhow!(
                    "rule id `{rule_id}` has prefix `{first}` but --dep is `{dep}`"
                ));
            }
            Ok((dep.to_string(), rule_id.to_string()))
        } else {
            Ok((dep.to_string(), format!("{dep}.{rule_id}")))
        }
    } else if let Some((dep, _rest)) = rule_id.split_once('.') {
        if dep.is_empty() {
            return Err(anyhow!("rule id `{rule_id}` is missing the dep prefix"));
        }
        Ok((dep.to_string(), rule_id.to_string()))
    } else {
        Err(anyhow!(
            "rule id `{rule_id}` must be `<dep>.<rule-name>`, or pass --dep <name>"
        ))
    }
}

fn destination_path(project_dir: &Path, personal: bool, language: &str, dep: &str) -> PathBuf {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let base = if personal {
        paths.personal_rules_dir
    } else {
        paths.project_rules_dir
    };
    let directory = if language == crate::types::ALL_LANGUAGE_META {
        crate::types::SHARED_LANGUAGE_DIR
    } else {
        language
    };
    base.join(directory).join(format!("{dep}.yaml"))
}

fn normalize_rule_language(language: &str) -> Result<&'static str> {
    crate::types::normalize_language_or_meta(language, &[crate::types::ALL_LANGUAGE_META])
        .ok_or_else(|| {
            anyhow!(
                "invalid language `{language}`. Must be one of: {}",
                crate::types::supported_language_display_list(&[crate::types::ALL_LANGUAGE_META])
            )
        })
}

fn read_yaml_mapping(path: &Path) -> Result<Mapping> {
    let text =
        fs::read_to_string(path).map_err(|e| anyhow!("failed to read {}: {e}", path.display()))?;
    let value: YamlValue = serde_yaml::from_str(&text)
        .map_err(|e| anyhow!("failed to parse {} as YAML: {e}", path.display()))?;
    match value {
        YamlValue::Mapping(m) => Ok(m),
        _ => Err(anyhow!("{} must be a YAML mapping", path.display())),
    }
}

fn collect_existing_rule_ids(project_dir: &Path) -> HashSet<String> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut out = HashSet::new();
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = load_rule_files(dir);
        for lrf in files {
            for rule in &lrf.rule_file.rules {
                if !rule.id.is_empty() {
                    out.insert(rule.id.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_id, validate_enum, AddOptions, EnforcementMode, VALID_SEVERITIES};
    use serde_json::Value;
    use std::collections::BTreeMap;

    #[test]
    fn parse_id_qualified() {
        let (dep, id) = parse_id("fastapi.async-routes", None).unwrap();
        assert_eq!(dep, "fastapi");
        assert_eq!(id, "fastapi.async-routes");
    }

    #[test]
    fn parse_id_unqualified_with_dep() {
        let (dep, id) = parse_id("async-routes", Some("fastapi")).unwrap();
        assert_eq!(dep, "fastapi");
        assert_eq!(id, "fastapi.async-routes");
    }

    #[test]
    fn parse_id_unqualified_without_dep_errors() {
        assert!(parse_id("async-routes", None).is_err());
    }

    #[test]
    fn parse_id_mismatched_dep_errors() {
        assert!(parse_id("fastapi.async-routes", Some("react")).is_err());
    }

    #[test]
    fn severity_validation() {
        assert!(validate_enum("severity", "must", VALID_SEVERITIES).is_ok());
        assert!(validate_enum("severity", "always", VALID_SEVERITIES).is_err());
    }

    #[test]
    fn add_lint_backed_rule_persists_structured_binding() {
        let tmp =
            std::env::temp_dir().join(format!("wh_rule_authoring_lint_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        super::add(
            &tmp,
            AddOptions {
                rule_id: "custom.mutable-defaults".into(),
                description: "Mutable defaults must be rejected".into(),
                severity: "must".into(),
                confidence: "high".into(),
                category: "default".into(),
                language: "python".into(),
                source_url: None,
                dep: Some("custom".into()),
                enforcement: EnforcementMode::Lint {
                    tool: "ruff".into(),
                    code: "B006".into(),
                },
                personal: true,
            },
        )
        .unwrap();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].signals[0]
                .lint
                .as_ref()
                .map(|lint| lint.tool.as_str()),
            Some("ruff")
        );
        assert_eq!(
            rules[0].signals[0]
                .lint
                .as_ref()
                .map(|lint| lint.code.as_str()),
            Some("B006")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn add_test_backed_rule_persists_linked_test() {
        let tmp =
            std::env::temp_dir().join(format!("wh_rule_authoring_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        super::add(
            &tmp,
            AddOptions {
                rule_id: "custom.snapshot-contract".into(),
                description: "Snapshots must stay covered".into(),
                severity: "should".into(),
                confidence: "high".into(),
                category: "convention".into(),
                language: "typescript".into(),
                source_url: None,
                dep: Some("custom".into()),
                enforcement: EnforcementMode::Test {
                    runner: "vitest".into(),
                    path: "tests/render/output.test.ts".into(),
                    selector: Some("snapshot_contract".into()),
                },
                personal: true,
            },
        )
        .unwrap();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].tests.len(), 1);
        assert_eq!(rules[0].tests[0].runner, "vitest");
        assert_eq!(rules[0].tests[0].path, "tests/render/output.test.ts");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn add_accepts_language_aliases_and_normalizes_storage() {
        let tmp =
            std::env::temp_dir().join(format!("wh_rule_authoring_alias_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        super::add(
            &tmp,
            AddOptions {
                rule_id: "custom.render-snapshot".into(),
                description: "Snapshots must stay covered".into(),
                severity: "should".into(),
                confidence: "high".into(),
                category: "convention".into(),
                language: "js".into(),
                source_url: None,
                dep: Some("custom".into()),
                enforcement: EnforcementMode::Test {
                    runner: "vitest".into(),
                    path: "tests/render/output.test.ts".into(),
                    selector: Some("snapshot_contract".into()),
                },
                personal: true,
            },
        )
        .unwrap();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].language, "javascript");
        assert_eq!(rules[0].languages, vec!["javascript"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn add_validator_backed_rule_persists_validators() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_rule_authoring_validator_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        super::add(
            &tmp,
            AddOptions {
                rule_id: "custom.inline-handlers".into(),
                description: "Inline handlers should be checked by a custom validator".into(),
                severity: "should".into(),
                confidence: "high".into(),
                category: "convention".into(),
                language: "html".into(),
                source_url: None,
                dep: Some("custom".into()),
                enforcement: EnforcementMode::Validator {
                    adapter: "command".into(),
                    rule: "custom.inline-handlers".into(),
                    config: BTreeMap::from([(
                        "path".into(),
                        Value::String("scripts/check-inline-handlers.py".into()),
                    )]),
                },
                personal: true,
            },
        )
        .unwrap();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].validators.len(), 1);
        assert_eq!(rules[0].validators[0].adapter, "command");
        assert_eq!(rules[0].validators[0].rule, "custom.inline-handlers");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
