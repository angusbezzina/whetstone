//! Report generation — integrated human-readable project summary.
//!
//! Composes `wh status` (adherence + rule-system scores), the top-10
//! `wh check` violations, and the last refresh-diff drift summary into a
//! single one-page markdown report. Suitable for PR comments, issue
//! bodies, or a quick "what's the state of my repo?" read.
//!
//! Epic 3E theme B (observability) — closes `whetstone-hpq`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::{check, status};

/// How many violations to surface in the Top Violations section.
const TOP_VIOLATIONS: usize = 10;

/// Marker used by the PR-comment flavor so consumers can find/update the
/// previous comment (mirrors `wh ci --pr-comment`'s `<!-- whetstone-ci-check -->`).
pub const PR_MARKER: &str = "<!-- whetstone-report -->";

pub struct ReportOptions<'a> {
    pub project_dir: &'a Path,
    pub pr_comment: bool,
}

/// Build the structured JSON envelope for the report. Non-JSON callers
/// render this via `to_markdown`.
pub fn build(opts: &ReportOptions<'_>) -> Result<Value> {
    let project_dir = opts.project_dir;

    // Rule-system + adherence (reuse status internals so we're consistent).
    // Skip drift check for speed; wh report is meant to be <3s.
    let status_result = status::compute_status(project_dir, false, false)?;

    // Violations (top-N by severity → count, then file/line).
    let scan_root = if project_dir.join("src").is_dir() {
        project_dir.join("src")
    } else {
        project_dir.to_path_buf()
    };
    let check_result = check::run(check::CheckOptions {
        project_dir,
        scan_paths: std::slice::from_ref(&scan_root),
        lang_filter: None,
        rule_filter: None,
    })?;
    let violations = rank_violations(&check_result);
    let top = violations
        .iter()
        .take(TOP_VIOLATIONS)
        .cloned()
        .collect::<Vec<Value>>();
    let violations_total = violations.len();

    // Drift: read the last refresh-diff.json if it exists.
    let refresh_diff = read_refresh_diff(project_dir);

    Ok(json!({
        "status": "ok",
        "project_dir": project_dir.display().to_string(),
        "rule_system_score": status_result.get("rule_system_score"),
        "adherence_score": status_result.get("adherence_score"),
        "adherence": status_result.get("adherence"),
        "label": status_result.get("label"),
        "rules_count": status_result
            .get("dimensions")
            .and_then(|d| d.get("rules_count")),
        "violations": {
            "total": violations_total,
            "top": top,
        },
        "drift": refresh_diff,
        "next_actions": build_next_actions(&status_result, violations_total, opts.pr_comment),
    }))
}

/// Render the JSON envelope as a markdown report. The leading PR marker is
/// always emitted so `--pr-comment` consumers can locate+update it; a plain
/// render adds the marker harmlessly as the first line.
pub fn to_markdown(data: &Value) -> String {
    let mut out = String::new();
    out.push_str(PR_MARKER);
    out.push('\n');
    out.push_str("# Whetstone Report\n\n");

    let adherence = data.get("adherence_score");
    let rule_sys = data.get("rule_system_score");
    let label = data.get("label").and_then(|v| v.as_str()).unwrap_or("unknown");
    let rules = data
        .get("rules_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    out.push_str(&format!("**Label:** {label}  \n"));
    out.push_str(&format!(
        "**Rule system:** {} / 100  \n",
        rule_sys
            .and_then(|v| v.as_i64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into())
    ));
    if let Some(adherence_score) = adherence.and_then(|v| v.as_i64()) {
        out.push_str(&format!("**Adherence:** {adherence_score} / 100  \n"));
    }
    out.push_str(&format!("**Approved rules:** {rules}\n\n"));

    // Adherence breakdown.
    if let Some(ad) = data.get("adherence").filter(|v| !v.is_null()) {
        out.push_str("## Adherence detail\n\n");
        let clean = ad.get("clean_ratio").and_then(|v| v.as_i64()).unwrap_or(0);
        let sev = ad
            .get("severity_component")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let eligible = ad
            .get("files_eligible")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let clean_files = ad
            .get("files_clean")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let v = ad.get("violations");
        let must = v.and_then(|o| o.get("must")).and_then(|x| x.as_i64()).unwrap_or(0);
        let should = v.and_then(|o| o.get("should")).and_then(|x| x.as_i64()).unwrap_or(0);
        let may = v.and_then(|o| o.get("may")).and_then(|x| x.as_i64()).unwrap_or(0);
        out.push_str(&format!(
            "- Clean files: **{clean_files} / {eligible}** ({clean}%)\n"
        ));
        out.push_str(&format!("- Severity-weighted: **{sev} / 100**\n"));
        out.push_str(&format!(
            "- Violations: **{must} must** · {should} should · {may} may\n\n"
        ));
    }

    // Top violations.
    let vi = data.get("violations");
    let total = vi.and_then(|v| v.get("total")).and_then(|x| x.as_i64()).unwrap_or(0);
    if total > 0 {
        out.push_str(&format!(
            "## Top {} violations (of {total})\n\n",
            TOP_VIOLATIONS.min(total as usize)
        ));
        if let Some(top) = vi.and_then(|v| v.get("top")).and_then(|t| t.as_array()) {
            out.push_str("| Severity | Rule | File | Line |\n");
            out.push_str("|---|---|---|---|\n");
            for item in top {
                let sev = item.get("severity").and_then(|s| s.as_str()).unwrap_or("");
                let rule = item
                    .get("rule_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let file = item.get("file").and_then(|s| s.as_str()).unwrap_or("");
                let line = item
                    .get("line")
                    .and_then(|s| s.as_u64())
                    .unwrap_or(0);
                out.push_str(&format!(
                    "| {sev} | `{rule}` | `{file}` | {line} |\n"
                ));
            }
            out.push('\n');
        }
    } else {
        out.push_str("## Violations\n\nNo violations detected. Nice.\n\n");
    }

    // Drift.
    if let Some(d) = data.get("drift").filter(|v| !v.is_null()) {
        let drifted = d
            .get("drift_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if drifted > 0 {
            out.push_str(&format!(
                "## Dependency drift\n\n{drifted} dep(s) have changed since the last refresh. \
                 Run `wh reinit` to re-resolve, then `wh extract` + `wh approve` \
                 to update rules.\n\n"
            ));
        } else {
            out.push_str("## Dependency drift\n\nNo drift. Rules are current.\n\n");
        }
    }

    // Next actions.
    if let Some(actions) = data.get("next_actions").and_then(|v| v.as_array()) {
        if !actions.is_empty() {
            out.push_str("## Next actions\n\n");
            for a in actions {
                let text = a
                    .get("message")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let cmd = a.get("command").and_then(|s| s.as_str()).unwrap_or("");
                if cmd.is_empty() {
                    out.push_str(&format!("- {text}\n"));
                } else {
                    out.push_str(&format!("- {text} — `{cmd}`\n"));
                }
            }
            out.push('\n');
        }
    }

    out.push_str("---\n");
    out.push_str("*Generated by Whetstone.*\n");
    out
}

pub fn default_report_path(project_dir: &Path) -> PathBuf {
    project_dir.join("whetstone").join("report.md")
}

pub fn write_markdown_report(project_dir: &Path, markdown: &str) -> Result<PathBuf> {
    let path = default_report_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, markdown)?;
    Ok(path)
}

// ── helpers ──

fn rank_violations(check_result: &Value) -> Vec<Value> {
    let Some(arr) = check_result.get("violations").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = arr.clone();
    out.sort_by(|a, b| {
        let rank = |v: &Value| match v.get("severity").and_then(|s| s.as_str()).unwrap_or("") {
            "must" => 0,
            "should" => 1,
            "may" => 2,
            _ => 3,
        };
        rank(a).cmp(&rank(b)).then_with(|| {
            a.get("file")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .cmp(b.get("file").and_then(|s| s.as_str()).unwrap_or(""))
        })
    });
    out
}

fn read_refresh_diff(project_dir: &Path) -> Value {
    let path: PathBuf = project_dir
        .join("whetstone")
        .join(".state")
        .join("refresh-diff.json");
    if !path.exists() {
        return Value::Null;
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

fn build_next_actions(
    status: &Value,
    violations_total: usize,
    _pr_comment: bool,
) -> Vec<Value> {
    let mut out = Vec::new();
    let adherence = status.get("adherence_score").and_then(|v| v.as_i64());
    match adherence {
        Some(a) if a < 80 => {
            out.push(json!({
                "message": format!("Adherence is {a}/100. Fix top violations."),
                "command": "wh scan src/ --json",
            }));
        }
        None => {
            out.push(json!({
                "message": "No adherence score: either no approved rules or no eligible files.",
                "command": "wh extract",
            }));
        }
        _ => {}
    }
    if violations_total == 0 {
        if let Some(inherited) = status.get("next_command").and_then(|c| c.as_str()) {
            out.push(json!({
                "message": "Everything clean. Follow the status recommendation.",
                "command": inherited,
            }));
        }
    }
    out
}
