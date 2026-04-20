//! `wh rule add` and `wh rule edit` — direct rule authoring and mutation.
//!
//! Covers Epic 3E theme C (authoring shortcuts):
//! - `wh rule add` lets users write a personal preference in one command,
//!   skipping the extract/submit/approve dance. Rules land as
//!   `status: approved` directly (user is the author AND the approver).
//! - `wh rule edit` bumps severity / confidence on existing approved rules
//!   as taste matures. Bulk via `--all` + selectors.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::load_rule_files;

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
const VALID_LANGUAGES: &[&str] = &["python", "typescript", "rust"];

// ── add ──

pub struct AddOptions<'a> {
    /// Full id (`dep.rule-name`) OR just `rule-name` with `dep` supplied.
    pub rule_id: &'a str,
    pub description: &'a str,
    pub match_regex: Option<&'a str>,
    pub severity: &'a str,
    pub confidence: &'a str,
    pub category: &'a str,
    pub language: &'a str,
    pub source_url: Option<&'a str>,
    pub dep: Option<&'a str>,
    /// Target the personal layer (gitignored) rather than the committed project layer.
    pub personal: bool,
}

pub fn add(project_dir: &Path, opts: AddOptions<'_>) -> Result<Value> {
    validate_enum("severity", opts.severity, VALID_SEVERITIES)?;
    validate_enum("confidence", opts.confidence, VALID_CONFIDENCES)?;
    validate_enum("category", opts.category, VALID_CATEGORIES)?;
    validate_enum("language", opts.language, VALID_LANGUAGES)?;

    if opts.description.trim().is_empty() {
        return Err(anyhow!("--description is required and must be non-empty"));
    }

    let (dep, full_id) = parse_id(opts.rule_id, opts.dep)?;
    let existing = collect_existing_rule_ids(project_dir);
    if existing.contains(&full_id) {
        return Err(anyhow!(
            "rule id `{full_id}` already exists in the project ruleset. Edit it with `wh rule edit` or pick a different id."
        ));
    }

    // Build the rule YAML mapping.
    let mut rule = Mapping::new();
    rule.insert(ystr("id"), ystr(&full_id));
    rule.insert(ystr("severity"), ystr(opts.severity));
    rule.insert(ystr("confidence"), ystr(opts.confidence));
    rule.insert(ystr("category"), ystr(opts.category));
    rule.insert(ystr("description"), ystr(opts.description));
    rule.insert(
        ystr("source_url"),
        ystr(opts
            .source_url
            .map(String::from)
            .unwrap_or_else(|| format!("personal://{dep}/{full_id}"))
            .as_str()),
    );
    rule.insert(ystr("approved"), YamlValue::Bool(true));
    rule.insert(ystr("status"), ystr("approved"));

    // Signals: at minimum one pattern signal when a match regex is given. If no
    // regex is supplied, write a placeholder lint_proxy signal so `wh validate`
    // accepts the rule but downstream check skips it until the user adds detail.
    let mut signals = Vec::new();
    if let Some(regex) = opts.match_regex {
        let mut sig = Mapping::new();
        sig.insert(ystr("id"), ystr("authored-pattern"));
        sig.insert(ystr("strategy"), ystr("pattern"));
        sig.insert(ystr("description"), ystr("Authored regex"));
        sig.insert(ystr("weight"), ystr("required"));
        sig.insert(ystr("match"), ystr(regex));
        signals.push(YamlValue::Mapping(sig));
    } else {
        let mut sig = Mapping::new();
        sig.insert(ystr("id"), ystr("authored-placeholder"));
        sig.insert(ystr("strategy"), ystr("lint_proxy"));
        sig.insert(
            ystr("description"),
            ystr("Placeholder — add a `match` regex via `wh rule edit` to make this enforceable"),
        );
        sig.insert(ystr("weight"), ystr("optional"));
        signals.push(YamlValue::Mapping(sig));
    }
    rule.insert(ystr("signals"), YamlValue::Sequence(signals));

    // Two golden examples: a pass+fail. The user can edit them later; this keeps
    // `wh validate` happy (it requires at least one example).
    let mut pass_ex = Mapping::new();
    pass_ex.insert(ystr("code"), ystr("// TODO: a code snippet that PASSES this rule"));
    pass_ex.insert(ystr("verdict"), ystr("pass"));
    pass_ex.insert(ystr("reason"), ystr("Adheres to the authored rule"));
    let mut fail_ex = Mapping::new();
    fail_ex.insert(ystr("code"), ystr("// TODO: a code snippet that FAILS this rule"));
    fail_ex.insert(ystr("verdict"), ystr("fail"));
    fail_ex.insert(ystr("reason"), ystr("Violates the authored rule"));
    rule.insert(
        ystr("golden_examples"),
        YamlValue::Sequence(vec![YamlValue::Mapping(pass_ex), YamlValue::Mapping(fail_ex)]),
    );

    // Append to the existing dep file when present, otherwise create it.
    let dest = destination_path(project_dir, opts.personal, opts.language, &dep);
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
        "next_command": "wh actions",
    }))
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
                            "rule `{id}` is a candidate; approve it via `wh approve` before editing"
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
            "no approved rules match the selector. Use `wh review` to inspect the ruleset."
        ));
    }

    Ok(json!({
        "status": "ok",
        "dry_run": dry_run,
        "changed": edits.iter().map(edit_record_to_json).collect::<Vec<_>>(),
        "count": edits.len(),
        "next_command": if dry_run { "wh rule edit <same args, without --dry-run>" } else { "wh actions" },
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
    base.join(language).join(format!("{dep}.yaml"))
}

fn read_yaml_mapping(path: &Path) -> Result<Mapping> {
    let text = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read {}: {e}", path.display()))?;
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
    use super::{parse_id, validate_enum, VALID_SEVERITIES};

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
}
