//! Raising a hand without stalling: a question for the owner is a Beads
//! issue labelled `human` in the repository's tracker (the owner sees it in
//! `bd human list`), and a `hand_raise` receipt in Whetstone's store. Answers
//! go back through `bd human respond` and are recorded as `hand_answer`
//! decisions. Whetstone wraps Beads; it never invents a second tracker.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::projection::HandStatus;

fn bd(project_root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("bd")
        .args(args)
        .current_dir(project_root)
        .env("BD_NON_INTERACTIVE", "1")
        .env_remove("BEADS_DIR")
        .output()
        .map_err(|error| format!("bd is unavailable: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap_or("bd failed")
            .to_string())
    }
}

/// Whether the repository has a Beads tracker to file questions in.
pub fn tracker_available(project_root: &Path) -> bool {
    project_root.join(".beads").is_dir()
}

/// File a human-needed issue and return its id.
pub fn file_issue(project_root: &Path, title: &str, description: &str) -> Result<String, String> {
    if !tracker_available(project_root) {
        return Err(
            "This repository has no Beads tracker (.beads) to file the question in; run bd init, or ask the owner directly.".into(),
        );
    }
    let title = title.chars().take(200).collect::<String>();
    let id = bd(
        project_root,
        &[
            "create",
            "--title",
            &title,
            "--labels",
            "human",
            "--type",
            "task",
            "--description",
            description,
            "--silent",
        ],
    )?;
    let id = id.lines().last().unwrap_or_default().trim().to_string();
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(format!("bd create did not return an issue id: {id}"));
    }
    Ok(id)
}

/// Queue work for any agent: a Beads task labelled `whetstone`, for
/// dashboard buttons when no bot (pstack's /make-bot-ui) is wired.
pub fn file_task(project_root: &Path, title: &str, description: &str) -> Result<String, String> {
    if !tracker_available(project_root) {
        return Err("This repository has no Beads tracker (.beads); run bd init.".into());
    }
    let id = bd(
        project_root,
        &[
            "create",
            "--title",
            title,
            "--labels",
            "whetstone-task",
            "--type",
            "task",
            "--description",
            description,
            "--silent",
        ],
    )?;
    Ok(id.lines().last().unwrap_or_default().trim().to_string())
}

/// Answer a human-needed issue in Beads (adds the comment and closes it).
pub fn respond(project_root: &Path, issue: &str, answer: &str) -> Result<(), String> {
    bd(
        project_root,
        &["human", "respond", issue, "--response", answer],
    )
    .map(|_| ())
}

/// The latest comment on an issue, when the owner answered it in Beads.
pub fn latest_comment(project_root: &Path, issue: &str) -> Option<String> {
    let text = bd(project_root, &["comments", issue, "--json"]).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .as_array()?
        .iter()
        .filter_map(|comment| comment.get("text").or_else(|| comment.get("body")))
        .filter_map(Value::as_str)
        .next_back()
        .map(str::to_owned)
}

/// Tracker status of every human-labelled issue, keyed by issue id.
pub fn statuses(project_root: &Path) -> BTreeMap<String, HandStatus> {
    let mut result = BTreeMap::new();
    if !tracker_available(project_root) {
        return result;
    }
    let Ok(text) = bd(
        project_root,
        &[
            "list", "--label", "human", "--all", "--limit", "0", "--json",
        ],
    ) else {
        return result;
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else {
        return result;
    };
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let answer = item
            .get("close_reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty() && *reason != "Responded")
            .map(str::to_owned);
        result.insert(id.to_string(), HandStatus { status, answer });
    }
    result
}

/// The issue text an agent files: the question, what it tried, what it
/// recommends, and the one decision needed.
pub fn issue_description(
    question: &str,
    tried: &str,
    recommendation: &str,
    trigger: &str,
    rule: Option<&str>,
) -> String {
    let mut text = format!(
        "## Question\n\n{question}\n\n## What was tried\n\n{tried}\n\n## Recommendation\n\n{recommendation}\n\n## Why the agent stopped\n\nTrigger: {trigger}."
    );
    if let Some(rule) = rule {
        text.push_str(&format!(" Rule: {rule}."));
    }
    text.push_str("\n\nAnswer with `wh change --answer <this issue> --content \"...\"` (or `bd human respond`); the agent has moved on to other ready work.\n");
    text
}
