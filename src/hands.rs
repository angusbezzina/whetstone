//! Raising a hand without stalling: a question for the owner is a Beads
//! issue labelled `human` in the repository's tracker (the owner sees it in
//! `bd human list`), and a `hand_raise` receipt in Whetstone's store. Answers
//! go back as a Beads response (comment and close) and are recorded as `hand_answer`
//! decisions. Whetstone wraps Beads; it never invents a second tracker.
//!
//! With the memory switch naming a Virgil group, the same hand is a native
//! task in the group's channel (`virgil tasks create`), not a Beads issue.
//! [`route`] is the one place that decides; the triggers and the receipt are
//! the same either way.

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

/// Where a raised hand was filed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiledHand {
    /// The Beads issue id, or the Virgil task reference (`task:<ULID>`).
    pub issue: String,
    /// `None` for Beads; `virgil:<group>` for a task in a Virgil channel.
    pub route: Option<String>,
}

/// The one routing point for a raised hand: `bd human` standalone, a task
/// in the attached group's channel when the store switch names Virgil.
/// `virgil tasks` records every task as the daemon's principal's whoever
/// asked, so an agent's hand (`raised_by` names its host) says so in the
/// task's first line.
pub fn route(
    project_root: &Path,
    title: &str,
    description: &str,
    raised_by: Option<&str>,
) -> Result<FiledHand, String> {
    match crate::storage::read_lesson_config(project_root)? {
        crate::storage::StoreConfig::Beads => Ok(FiledHand {
            issue: file_issue(project_root, title, description)?,
            route: None,
        }),
        crate::storage::StoreConfig::Virgil => {
            let group = match crate::memory::virgil::attachment(project_root) {
                Ok(Some((group, _))) => group,
                Ok(None) => return Err("This checkout is not attached to a Virgil repository, so the hand has no channel to go to. Attach it with virgil repos attach <repository> --group <group>.".into()),
                Err(refusal) => return Err(refusal.message),
            };
            let title = title.chars().take(200).collect::<String>();
            let description = match raised_by {
                Some(host) => {
                    format!("Raised by {host} (an agent) through Whetstone.\n\n{description}")
                }
                None => description.to_string(),
            };
            let task =
                crate::memory::virgil::create_task(project_root, &group, &title, &description)
                    .map_err(|refusal| refusal.message)?;
            Ok(FiledHand {
                issue: task,
                route: Some(format!("virgil:{group}")),
            })
        }
    }
}

/// The Virgil group a hand's route names, if it went to Virgil.
pub fn virgil_group(route: Option<&str>) -> Option<&str> {
    route.and_then(|route| route.strip_prefix("virgil:"))
}

/// Tracker status of hands filed as Virgil tasks, keyed by task reference.
pub fn virgil_statuses(
    project_root: &Path,
    hands: &[(String, String)],
) -> BTreeMap<String, HandStatus> {
    let mut result = BTreeMap::new();
    for (task, group) in hands.iter().take(32) {
        if let Some((state, answer)) = crate::memory::virgil::task_status(project_root, group, task)
        {
            let status = match state.as_str() {
                "done" | "cancelled" => "closed".to_string(),
                other => other.to_string(),
            };
            result.insert(task.clone(), HandStatus { status, answer });
        }
    }
    result
}

/// Answer a hand where it was filed.
pub fn answer(
    project_root: &Path,
    issue: &str,
    route: Option<&str>,
    answer: &str,
) -> Result<(), String> {
    match virgil_group(route) {
        Some(group) => crate::memory::virgil::answer_task(project_root, group, issue, answer)
            .map_err(|refusal| refusal.message),
        None => respond(project_root, issue, answer),
    }
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
/// Writes what `bd human respond` writes, through commands that also work on
/// bd 1.1.2, whose `human respond` cannot open the store.
pub fn respond(project_root: &Path, issue: &str, answer: &str) -> Result<(), String> {
    bd(
        project_root,
        &["comments", "add", issue, &format!("Response: {answer}")],
    )?;
    bd(project_root, &["close", issue, "--reason", "Responded"]).map(|_| ())
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
