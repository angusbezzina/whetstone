//! The Virgil backend: lessons in the group's Records `lesson` collection
//! and raised hands as tasks in the group's channel, reached only through the
//! `virgil` CLI on the daemon socket. No MCP tool, no door, no database.
//!
//! Every call is `virgil <lessons|tasks> <op> --group <G> --input -` with one
//! JSON request on stdin, answered by one JSON line on stdout:
//! `{"api_version":"virgil.lessons/v1","result":{...}}` or
//! `{"api_version":...,"error":{"code","message","hint"}}`, with exit codes 0
//! ok, 2 invalid request, 5 not permitted, 7 unavailable (the `virgil tasks`
//! conventions). `virgil lessons` ships with Off Beads step 2a; until then
//! this adapter is exercised against a fake `virgil` on PATH.
//!
//! Lesson results use Whetstone's lesson fields, with the daemon's
//! `provenance` in place of the stamp; the daemon stamps it and refuses a
//! request that supplies it, so requests never carry one.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::domain::{ForgetProposal, Lesson, LessonDecision, LessonStamp, Trust, LESSON_SCHEMA_V1};

use super::store::{LessonStore, StoredLesson};
use super::Refusal;

pub const LESSONS_API: &str = "virgil.lessons/v1";
pub const TASKS_API: &str = "virgil.tasks/v1";

/// The program to run: `virgil` on PATH.
fn program() -> PathBuf {
    PathBuf::from("virgil")
}

/// Run one `virgil` operation and return its `result`.
pub fn call(
    project_root: &Path,
    family: &str,
    op: &str,
    group: &str,
    args: &[&str],
    request: &Value,
) -> Result<Value, Refusal> {
    let mut child = Command::new(program())
        .arg(family)
        .arg(op)
        .args(args)
        .args(["--group", group, "--input", "-"])
        .current_dir(project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Refusal::new(
                "store_unavailable",
                format!("virgil is not available ({error}); the memory switch names Virgil, so nothing was written. Install or start Virgil, or switch back with wh init --action memory --store beads."),
            )
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(request.to_string().as_bytes());
    }
    let output = child.wait_with_output().map_err(|error| {
        Refusal::new(
            "store_unavailable",
            format!("virgil did not answer: {error}"),
        )
    })?;
    let line = String::from_utf8_lossy(&output.stdout);
    let answer = line
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .unwrap_or(Value::Null);
    if let Some(error) = answer.get("error") {
        let message = error["message"]
            .as_str()
            .unwrap_or("virgil refused the request");
        let hint = error["hint"]
            .as_str()
            .map(|hint| format!(" ({hint})"))
            .unwrap_or_default();
        let code = match output.status.code() {
            Some(5) => "not_permitted",
            Some(7) => "store_unavailable",
            Some(2) => "invalid_request",
            _ => "store_refused",
        };
        return Err(Refusal::new(
            code,
            format!("virgil {family} {op}: {message}{hint}"),
        ));
    }
    if !output.status.success() {
        let code = if output.status.code() == Some(7) {
            "store_unavailable"
        } else {
            "store_refused"
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Refusal::new(
            code,
            format!(
                "virgil {family} {op} failed: {}",
                stderr
                    .lines()
                    .last()
                    .unwrap_or("no detail")
                    .chars()
                    .take(300)
                    .collect::<String>()
            ),
        ));
    }
    answer.get("result").cloned().ok_or_else(|| {
        Refusal::new(
            "store_contract",
            format!("virgil {family} {op} answered without a result line"),
        )
    })
}

/// A lesson as Virgil returns it, read into Whetstone's record.
pub fn lesson_from_virgil(value: &Value) -> Result<StoredLesson, Refusal> {
    let mut object = value.as_object().cloned().ok_or_else(|| {
        Refusal::new(
            "store_contract",
            "virgil returned a lesson that is not an object",
        )
    })?;
    let id = object
        .remove("id")
        .and_then(|id| id.as_str().map(str::to_owned));
    let revision = object
        .remove("revision")
        .and_then(|revision| revision.as_u64())
        .unwrap_or(1);
    if let Some(provenance) = object.remove("provenance") {
        object.insert("stamp".into(), provenance);
    }
    // Anchors may come as `{repo, path, sha256}` (hex), the shape Messo's
    // lesson port reads, or as Whetstone's `{path, digest}`.
    if let Some(Value::Array(anchors)) = object.get_mut("anchors") {
        for anchor in anchors.iter_mut() {
            if let Some(anchor) = anchor.as_object_mut() {
                anchor.remove("repo");
                if let Some(hex) = anchor.remove("sha256") {
                    if !anchor.contains_key("digest") {
                        if let Some(hex) = hex.as_str() {
                            anchor.insert("digest".into(), json!(format!("sha256:{hex}")));
                        }
                    }
                }
            }
        }
    }
    object.insert("schema".into(), json!(LESSON_SCHEMA_V1));
    let lesson = serde_json::from_value::<Lesson>(Value::Object(object)).map_err(|error| {
        Refusal::new(
            "store_contract",
            format!("virgil returned a lesson Whetstone cannot read: {error}"),
        )
    })?;
    lesson.validate().map_err(|error| {
        Refusal::new(
            "store_contract",
            format!("virgil returned a lesson that breaks the contract: {error}"),
        )
    })?;
    Ok(StoredLesson {
        locator: format!(
            "virgil:{}",
            id.unwrap_or_else(|| format!("lesson/{}", lesson.key))
        ),
        lesson,
        revision,
        reference: None,
    })
}

fn lessons_of(result: &Value) -> Result<Vec<StoredLesson>, Refusal> {
    let mut lessons = result["lessons"]
        .as_array()
        .ok_or_else(|| Refusal::new("store_contract", "virgil returned no lessons list"))?
        .iter()
        .map(lesson_from_virgil)
        .collect::<Result<Vec<_>, _>>()?;
    lessons.sort_by(|left, right| left.lesson.key.cmp(&right.lesson.key));
    Ok(lessons)
}

fn one_lesson(result: &Value) -> Result<StoredLesson, Refusal> {
    let mut stored = lesson_from_virgil(&result["lesson"])?;
    // The daemon's receipt for a person's decision is the decision's record.
    if let (Some(receipt), Some(decision)) =
        (result["receipt"].as_str(), stored.lesson.decision.as_mut())
    {
        if decision.receipt.is_none() {
            decision.receipt = Some(receipt.to_string());
        }
    }
    Ok(stored)
}

/// The writer's lesson as a request: never the stamp, trust, status or
/// decision, which the daemon owns.
fn lesson_request(lesson: &Lesson) -> Value {
    json!({
        "key": lesson.key,
        "text": lesson.text,
        "about": lesson.about,
        "concepts": lesson.concepts,
        "audience": lesson.audience,
        "anchors": lesson.anchors,
        "supersedes": lesson.supersedes,
    })
}

pub struct VirgilLessons {
    root: PathBuf,
    group: String,
}

impl VirgilLessons {
    pub fn new(project_root: &Path, group: &str) -> Self {
        Self {
            root: project_root.to_path_buf(),
            group: group.to_string(),
        }
    }

    fn lessons(&self, op: &str, request: Value) -> Result<Value, Refusal> {
        call(&self.root, "lessons", op, &self.group, &[], &request)
    }
}

impl LessonStore for VirgilLessons {
    fn backend(&self) -> &'static str {
        "virgil"
    }

    fn find(&self) -> Result<Vec<StoredLesson>, Refusal> {
        lessons_of(&self.lessons("find", json!({}))?)
    }

    fn proposals(&self) -> Result<Vec<StoredLesson>, Refusal> {
        lessons_of(&self.lessons("proposals", json!({}))?)
    }

    fn remember(&self, lesson: &Lesson, idempotency: &str) -> Result<StoredLesson, Refusal> {
        one_lesson(&self.lessons(
            "remember",
            json!({"lesson": lesson_request(lesson), "request_id": idempotency}),
        )?)
    }

    fn admit(
        &self,
        key: &str,
        trust: Trust,
        decision: &LessonDecision,
        _stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        one_lesson(&self.lessons(
            "admit",
            json!({
                "key": key,
                "trust": trust,
                "rationale": decision.rationale,
                "request_id": idempotency,
            }),
        )?)
    }

    fn supersede(
        &self,
        key: &str,
        correction: &Lesson,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        one_lesson(&self.lessons(
            "supersede",
            json!({"key": key, "lesson": lesson_request(correction), "request_id": idempotency}),
        )?)
    }

    fn forget(
        &self,
        key: &str,
        decision: Option<&LessonDecision>,
        proposal: Option<&ForgetProposal>,
        _stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        let reason = decision
            .map(|decision| decision.rationale.clone())
            .or_else(|| proposal.map(|proposal| proposal.reason.clone()))
            .unwrap_or_default();
        one_lesson(&self.lessons(
            "forget",
            json!({
                "key": key,
                "reason": reason,
                "propose": decision.is_none(),
                "request_id": idempotency,
            }),
        )?)
    }
}

/// File a raised hand as a native task in the group's channel; returns the
/// task reference (`task:<ULID>`).
pub fn create_task(
    project_root: &Path,
    group: &str,
    title: &str,
    description: &str,
) -> Result<String, Refusal> {
    let result = call(
        project_root,
        "tasks",
        "create",
        group,
        &[],
        &json!({"scope": {"group": group}, "title": title, "description": description}),
    )?;
    result["task"]
        .as_str()
        .filter(|task| !task.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Refusal::new("store_contract", "virgil tasks create returned no task"))
}

/// A task's state and its latest comment, from this device's replica.
pub fn task_status(
    project_root: &Path,
    group: &str,
    task: &str,
) -> Option<(String, Option<String>)> {
    let result = call(project_root, "tasks", "show", group, &[task], &json!({})).ok()?;
    let task = result
        .get("task")
        .filter(|task| task.is_object())
        .unwrap_or(&result);
    let state = task["state"].as_str()?.to_string();
    let answer = task["comments"]
        .as_array()
        .and_then(|comments| comments.last())
        .and_then(|comment| {
            comment["text"]
                .as_str()
                .or_else(|| comment["body"].as_str())
        })
        .map(str::to_owned);
    Some((state, answer))
}

/// Answer a raised hand's task: the answer as a comment, then done.
pub fn answer_task(
    project_root: &Path,
    group: &str,
    task: &str,
    answer: &str,
) -> Result<(), Refusal> {
    call(
        project_root,
        "tasks",
        "comment",
        group,
        &[task],
        &json!({"task": task, "text": answer}),
    )?;
    call(
        project_root,
        "tasks",
        "move",
        group,
        &[task],
        &json!({"task": task, "state": "done"}),
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_anchor_shape_reads_as_a_whetstone_anchor() {
        let hex = "a".repeat(64);
        let stored = lesson_from_virgil(&json!({
            "id": "lesson:01",
            "key": "values-are-floats",
            "text": "Values are floats.",
            "audience": {"kind": "group", "name": "G"},
            "provenance": {"who": "principal:P", "actor": "agent", "at": "2026-10-13T09:00:00Z"},
            "anchors": [{"repo": "gpu-cli/virgil", "path": "app.py", "sha256": hex}],
            "trust": "inferred",
            "status": "proposed",
        }))
        .expect("lesson");
        assert_eq!(
            stored.lesson.anchors[0]
                .digest
                .as_ref()
                .map(|digest| digest.as_str().to_string()),
            Some(format!("sha256:{hex}"))
        );
        assert_eq!(stored.locator, "virgil:lesson:01");
    }
}
