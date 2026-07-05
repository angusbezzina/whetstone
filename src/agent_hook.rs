//! Agent-harness hook adapters (whetstone-cpt).
//!
//! The highest-leverage enforcement point: instead of catching rule violations
//! post-hoc in CI, a Claude Code **PostToolUse** hook scans the file the agent
//! just edited and feeds any violations straight back into the same turn, so the
//! agent fixes them before moving on.
//!
//! Protocol (https://code.claude.com/docs/en/hooks): the hook receives the event
//! JSON on stdin (`tool_input.file_path`, `cwd`, ...) and responds on stdout:
//!   - advisory  → `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":...}}`
//!   - blocking  → `{"decision":"block","reason":...}`
//!
//! Both exit 0. We fail OPEN: any parse/scan/infra error exits 0 with no output,
//! so a Whetstone hiccup never wedges the agent.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::check::{self, CheckOptions};

/// Read a PostToolUse event from stdin, scan the edited file, and emit the hook
/// response. Returns the process exit code (always 0 — blocking is signalled via
/// the JSON `decision`, not the exit code).
pub fn post_tool_use(project_dir: &Path, blocking: bool) -> Result<i32> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let event: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return Ok(0), // not our JSON — stay out of the way
    };
    if let Some(resp) = handle_event(&event, project_dir, blocking) {
        if let Ok(s) = serde_json::to_string(&resp) {
            println!("{s}");
        }
    }
    Ok(0)
}

/// Core, IO-free logic: given a PostToolUse event, return the hook response JSON
/// to emit, or `None` when there is nothing to say (clean file, non-source file,
/// missing/absent path, or a scan error — all silent).
pub fn handle_event(event: &Value, project_dir: &Path, blocking: bool) -> Option<Value> {
    let file_path = event
        .get("tool_input")
        .and_then(|ti| ti.get("file_path"))
        .and_then(|v| v.as_str())?;

    // Resolve the project root: an explicit --project-dir wins; otherwise the
    // hook's `cwd` (Claude Code runs hooks from the project dir); else ".".
    let base: PathBuf = if project_dir.as_os_str() == "." {
        event
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| project_dir.to_path_buf())
    } else {
        project_dir.to_path_buf()
    };

    let target = {
        let p = Path::new(file_path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        }
    };
    if !target.exists() {
        return None;
    }

    let scan_paths = [target];
    let result = check::run(CheckOptions {
        project_dir: &base,
        scan_paths: &scan_paths,
        lang_filter: None,
        rule_filter: None,
        injected_packs: &[],
    })
    .ok()?;

    let violations = result.get("violations").and_then(|v| v.as_array())?;
    if violations.is_empty() {
        return None; // clean — say nothing
    }

    let feedback = format_feedback(file_path, violations);
    Some(if blocking {
        json!({ "decision": "block", "reason": feedback })
    } else {
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": feedback,
            }
        })
    })
}

fn format_feedback(file_path: &str, violations: &[Value]) -> String {
    let mut lines = vec![format!(
        "Whetstone: {} rule violation(s) in {} — please fix before continuing:",
        violations.len(),
        file_path
    )];
    for v in violations {
        let rid = v.get("rule_id").and_then(|x| x.as_str()).unwrap_or("?");
        let sev = v.get("severity").and_then(|x| x.as_str()).unwrap_or("");
        let line = v.get("line").and_then(|x| x.as_i64()).unwrap_or(0);
        let desc = v
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let url = v.get("source_url").and_then(|x| x.as_str()).unwrap_or("");
        let cite = if url.is_empty() {
            String::new()
        } else {
            format!(" ({url})")
        };
        lines.push(format!("  - [{sev}] {rid} at line {line}: {desc}{cite}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp project whose only rule flags a bare `.unwrap()` via ast_scope.
    fn project_with_unwrap_rule() -> PathBuf {
        // Per-call atomic counter so sibling tests never share a dir (a nanos
        // collision under parallelism previously let one test delete another's
        // fixture mid-scan).
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "wh_hook_{}_{}_{seq}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let rules = tmp.join("whetstone").join("rules").join("rust");
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::write(
            rules.join("demo.yaml"),
            "source:\n  name: demo\nrules:\n  - id: demo.no-unwrap\n    severity: should\n    confidence: high\n    category: convention\n    description: Avoid bare unwrap in demo code\n    source_url: https://example.com/unwrap\n    approved: true\n    status: approved\n    signals:\n      - id: s\n        strategy: ast\n        weight: required\n        ast_query: '((call_expression function: (field_expression field: (field_identifier) @m (#eq? @m \"unwrap\"))) @match)'\n    golden_examples:\n      - code: \"fn f(){ let _ = x.unwrap(); }\"\n        verdict: fail\n        reason: bare unwrap\n      - code: \"fn f(){ let _ = x.expect(\\\"y\\\"); }\"\n        verdict: pass\n        reason: expect ok\n",
        )
        .unwrap();
        tmp
    }

    fn event_for(tmp: &Path, rel_file: &str) -> Value {
        json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "cwd": tmp.to_string_lossy(),
            "tool_input": { "file_path": rel_file }
        })
    }

    #[test]
    fn violation_produces_advisory_context() {
        let tmp = project_with_unwrap_rule();
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/bad.rs"), "fn f() { let _ = y.unwrap(); }\n").unwrap();
        let resp = handle_event(&event_for(&tmp, "src/bad.rs"), Path::new("."), false)
            .expect("expected advisory response");
        let ctx = resp["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("demo.no-unwrap"), "{ctx}");
        assert!(ctx.contains("src/bad.rs"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn blocking_mode_uses_decision_block() {
        let tmp = project_with_unwrap_rule();
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/bad.rs"), "fn f() { let _ = y.unwrap(); }\n").unwrap();
        let resp =
            handle_event(&event_for(&tmp, "src/bad.rs"), Path::new("."), true).expect("expected block");
        assert_eq!(resp["decision"], "block");
        assert!(resp["reason"].as_str().unwrap().contains("demo.no-unwrap"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clean_file_is_silent() {
        let tmp = project_with_unwrap_rule();
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/ok.rs"), "fn f() { let _ = y.expect(\"z\"); }\n").unwrap();
        assert!(handle_event(&event_for(&tmp, "src/ok.rs"), Path::new("."), false).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_path_and_bad_input_are_silent() {
        let tmp = project_with_unwrap_rule();
        // no file_path
        assert!(handle_event(&json!({"tool_input":{}}), Path::new("."), false).is_none());
        // path that does not exist
        assert!(handle_event(&event_for(&tmp, "src/nope.rs"), Path::new("."), false).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
