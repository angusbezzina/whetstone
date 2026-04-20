//! `wh approve` — flip candidate rules to approved.
//!
//! Replaces the old `wh apply --approve` flow. Two modes:
//!
//! - `wh approve <rule-id>` — flip one rule.
//! - `wh approve --all [--dep <name>] [--confidence <level>]` — flip every
//!   matching candidate in the project ruleset.
//!
//! The mutation is line-based, preserving comments and indentation in the
//! underlying YAML. Only `status:` and `approved:` lines are touched; no
//! other fields are added or removed.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::layers::LayerPaths;
use crate::rules::{load_rule_files, LoadedRuleFile};

/// Flip a single candidate rule to `status: approved`, `approved: true`.
pub fn approve_by_id(project_dir: &Path, rule_id: &str) -> Result<Value> {
    let found = find_rule(project_dir, rule_id)?;
    if is_already_approved(&found, rule_id) {
        return Ok(json!({
            "status": "ok",
            "rule_id": rule_id,
            "action": "noop",
            "reason": "already approved",
            "file": found.file_path,
        }));
    }
    let changed = rewrite_rule_to_approved(&PathBuf::from(&found.file_path), rule_id)?;
    if !changed {
        return Err(anyhow!(
            "rule `{rule_id}` found but approval lines could not be rewritten in {}",
            found.file_path
        ));
    }
    Ok(json!({
        "status": "ok",
        "rule_id": rule_id,
        "action": "approved",
        "file": found.file_path,
    }))
}

/// Flip every candidate rule matching the optional filters.
pub fn approve_bulk(
    project_dir: &Path,
    dep_filter: Option<&str>,
    confidence_filter: Option<&str>,
) -> Result<Value> {
    let paths = LayerPaths::for_project(project_dir);
    let (files, _) = load_rule_files(&paths.project_rules_dir);

    let mut targets: Vec<(PathBuf, String)> = Vec::new();
    for lrf in &files {
        let source_name = lrf.rule_file.source.name.as_str();
        if let Some(dep) = dep_filter {
            if !source_name.eq_ignore_ascii_case(dep) {
                continue;
            }
        }
        for rule in &lrf.rule_file.rules {
            // Only move candidates.
            let is_candidate = rule.status.as_deref() == Some("candidate")
                || (rule.status.is_none() && !rule.approved);
            if !is_candidate {
                continue;
            }
            if let Some(level) = confidence_filter {
                match rule.confidence.as_deref() {
                    Some(c) if c.eq_ignore_ascii_case(level) => {}
                    _ => continue,
                }
            }
            if rule.id.is_empty() {
                continue;
            }
            targets.push((PathBuf::from(&lrf.file_path), rule.id.clone()));
        }
    }

    let mut approved_ids: Vec<String> = Vec::new();
    let mut touched_files: Vec<String> = Vec::new();
    for (path, rule_id) in &targets {
        let changed = rewrite_rule_to_approved(path, rule_id)?;
        if changed {
            approved_ids.push(rule_id.clone());
            let as_str = path.display().to_string();
            if !touched_files.contains(&as_str) {
                touched_files.push(as_str);
            }
        }
    }

    Ok(json!({
        "status": "ok",
        "filter": {
            "dep": dep_filter,
            "confidence": confidence_filter,
        },
        "approved_count": approved_ids.len(),
        "approved_ids": approved_ids,
        "files_touched": touched_files,
    }))
}

// ── Internals ──

struct FoundRule {
    file_path: String,
    file: crate::rules::RuleFile,
}

fn find_rule(project_dir: &Path, rule_id: &str) -> Result<FoundRule> {
    let paths = LayerPaths::for_project(project_dir);
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        let (files, _) = load_rule_files(dir);
        for lrf in files {
            if lrf.rule_file.rules.iter().any(|r| r.id == rule_id) {
                return Ok(FoundRule {
                    file_path: lrf.file_path,
                    file: lrf.rule_file,
                });
            }
        }
    }
    Err(anyhow!(
        "rule `{rule_id}` not found under whetstone/rules/ or whetstone/.personal/rules/"
    ))
}

fn is_already_approved(found: &FoundRule, rule_id: &str) -> bool {
    found
        .file
        .rules
        .iter()
        .find(|r| r.id == rule_id)
        .map(|r| r.approved || r.status.as_deref() == Some("approved"))
        .unwrap_or(false)
}

/// Rewrite a rule block so `status:` is `approved` and `approved:` is `true`.
/// Missing fields are inserted at the end of the rule block.
///
/// Returns true if the file content changed, false otherwise.
fn rewrite_rule_to_approved(path: &Path, rule_id: &str) -> Result<bool> {
    // Silent check that the file still parses before we mutate.
    let text = fs::read_to_string(path)
        .map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
    let _ = serde_yaml::from_str::<serde_yaml::Value>(&text)
        .map_err(|e| anyhow!("failed to parse {}: {e}", path.display()))?;

    let (block_start, block_end, field_indent) = match locate_rule_block(&text, rule_id) {
        Some(t) => t,
        None => {
            return Err(anyhow!(
                "rule `{rule_id}` missing in {} (did the file move?)",
                path.display()
            ));
        }
    };

    let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();

    let mut changed = false;
    for (key, value) in [("status", "approved"), ("approved", "true")] {
        let current_end = mut_block_end(&lines, block_start);
        if replace_field_in_block(&mut lines, block_start, current_end, field_indent, key, value) {
            changed = true;
        } else {
            let insert_at = skip_trailing_blank_lines(&lines, current_end);
            let last = insert_at.saturating_sub(1);
            if last < lines.len() && !lines[last].ends_with('\n') {
                lines[last].push('\n');
            }
            lines.insert(
                insert_at,
                format!("{}{key}: {value}\n", " ".repeat(field_indent)),
            );
            changed = true;
        }
    }
    let _ = block_end;

    if changed {
        fs::write(path, lines.concat())?;
    }
    Ok(changed)
}

fn locate_rule_block(text: &str, rule_id: &str) -> Option<(usize, usize, usize)> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let re = id_line_regex();
    let mut rule_start: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(caps) = re.captures(line) {
            let leading = caps.get(1).map(|m| m.as_str().len()).unwrap_or(0);
            let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if id == rule_id {
                rule_start = Some((i, leading));
                break;
            }
        }
    }
    let (start, id_indent) = rule_start?;

    let mut field_indent = id_indent + 2;
    for line in &lines[start + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading > id_indent {
            field_indent = leading;
        }
        break;
    }

    let mut end = lines.len();
    for (offset, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading <= id_indent && !line.starts_with(' ') && !line.trim_start().starts_with('-') {
            end = offset;
            break;
        }
        if leading == id_indent && line.trim_start().starts_with("- ") {
            end = offset;
            break;
        }
    }

    Some((start, end, field_indent))
}

fn mut_block_end(lines: &[String], start: usize) -> usize {
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let id_indent = refs[start].chars().take_while(|c| *c == ' ').count();
    for (offset, line) in refs.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading <= id_indent && !line.starts_with(' ') && !line.trim_start().starts_with('-') {
            return offset;
        }
        if leading == id_indent && line.trim_start().starts_with("- ") {
            return offset;
        }
    }
    refs.len()
}

fn replace_field_in_block(
    lines: &mut [String],
    start: usize,
    end: usize,
    field_indent: usize,
    key: &str,
    value: &str,
) -> bool {
    let prefix = format!("{}{key}:", " ".repeat(field_indent));
    for line in lines.iter_mut().take(end).skip(start + 1) {
        let trimmed = line.trim_end_matches(&['\n', '\r'][..]);
        if trimmed.starts_with(&prefix) {
            let after_colon = &trimmed[prefix.len()..];
            let comment = extract_trailing_comment(after_colon);
            let tail = match comment {
                Some(c) => format!("  {c}"),
                None => String::new(),
            };
            let newline = if line.ends_with('\n') { "\n" } else { "" };
            let replacement = format!("{prefix} {value}{tail}{newline}");
            if *line == replacement {
                return true;
            }
            *line = replacement;
            return true;
        }
    }
    false
}

fn skip_trailing_blank_lines(lines: &[String], end: usize) -> usize {
    let mut i = end;
    while i > 0 && lines[i - 1].trim().is_empty() {
        i -= 1;
    }
    i
}

fn extract_trailing_comment(after_colon: &str) -> Option<&str> {
    let bytes = after_colon.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return Some(&after_colon[i..]);
        }
    }
    None
}

fn id_line_regex() -> &'static regex::Regex {
    static CELL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        regex::Regex::new(r#"^(\s*)- id:\s*['"]?([^'"\s]+)['"]?\s*$"#).expect("valid regex")
    })
}

// Keep the type alias public to prevent dead_code noise on the helper.
#[allow(dead_code)]
type LoadedFile = LoadedRuleFile;
