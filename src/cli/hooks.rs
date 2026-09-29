//! The hook entry points: the agent Stop hook and the Git pre-commit and pre-push gates.

use super::*;

/// The hook's JSON payload from stdin (bounded; empty when interactive).
pub(super) fn hook_input() -> serde_json::Value {
    use std::io::{IsTerminal, Read};
    let mut input = String::new();
    if !std::io::stdin().is_terminal() {
        let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut input);
    }
    serde_json::from_str::<serde_json::Value>(&input).unwrap_or(json!({}))
}

/// Per-session hook state under the private state root.
pub(super) fn session_file(project_dir: &Path, session: &str, kind: &str) -> Option<PathBuf> {
    ProjectLayout::resolve(project_dir).ok().map(|layout| {
        let digest = format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(session.as_bytes())
        );
        layout
            .state_root()
            .join("hooks")
            .join(format!("{kind}-{}", &digest[..16]))
    })
}

/// The session id a host puts in its hook payload.
pub(super) fn session_id(input: &serde_json::Value) -> Option<&str> {
    input["session_id"]
        .as_str()
        .or_else(|| input["conversation_id"].as_str())
}

/// Session start: remember the commit the session began at, so the stop hook
/// also proves work the agent committed during the session.
pub(super) fn remember_session_base(project_dir: &Path, input: &serde_json::Value) {
    let Some(session) = session_id(input) else {
        return;
    };
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(project_dir)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
    else {
        return;
    };
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || !is_commit_id(&head) {
        return;
    }
    if let Some(path) = session_file(project_dir, session, "session-base") {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, head);
    }
}

pub(super) fn is_commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|character| character.is_ascii_hexdigit())
}

/// The failing rules of a check, as a stable key for "the same finding".
pub(super) fn failing_key(value: &serde_json::Value) -> String {
    let mut failing = value["data"]["gates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|gate| gate["state"] == "fail")
        .filter_map(|gate| gate["id"].as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if value["data"]["report"]["state"] == "violated" && failing.is_empty() {
        failing.push("whetstone.native-scan".into());
    }
    failing.sort();
    failing.join(",")
}

/// An agent host's stop hook: run the change-scoped check and hand
/// violations back to the same session in the host's format. Bounded: when
/// the same rules still fail after two repairs, raise a hand for the owner
/// instead of a third attempt, and let the agent stop.
pub(super) fn run_stop_hook(
    service: &CommandService,
    mut request: CheckRequest,
    host: &str,
) -> i32 {
    let hook = hook_input();
    if host == "claude" && hook.get("cursor_version").is_some() {
        // Cursor also runs Claude hooks; its own stop hook serves it.
        return 0;
    }
    let session = session_id(&hook).unwrap_or("default").to_string();
    if request.base.is_none() {
        request.base = session_file(&request.project_dir, &session, "session-base")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| text.trim().to_string())
            .filter(|head| is_commit_id(head));
    }
    let project_dir = request.project_dir.clone();
    let state_file =
        session_file(&project_dir, &session, "stop").map(|path| path.with_extension("json"));
    let previous = state_file
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or(json!({}));
    let write_state = |value: &serde_json::Value| {
        if let Some(path) = &state_file {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, value.to_string());
        }
    };
    let response = service.execute(ServiceRequest::Check(request));
    let value = serde_json::to_value(&response).unwrap_or(json!({}));
    let (code, text) = crate::hosts::stop_feedback(&value);
    let emit = |block: bool, text: &str| -> i32 {
        let (code, stdout, stderr) = crate::hosts::stop_output(host, block, text);
        if !stdout.is_empty() {
            println!("{stdout}");
        }
        if !stderr.is_empty() {
            eprintln!("{stderr}");
        }
        code
    };
    if code != 2 {
        write_state(&json!({}));
        return emit(false, &text);
    }
    let key = failing_key(&value);
    let returns = if previous["key"] == key.as_str() {
        previous["returns"].as_u64().unwrap_or(0) + 1
    } else {
        1
    };
    if returns >= 3 && previous["hand"].is_null() {
        // Two repairs did not clear the same failures: the owner decides.
        let hand = HandRequest {
            question: format!(
                "The same checks still fail after two repair attempts ({key}). How should the agent proceed?"
            ),
            tried: format!(
                "Two repairs within the task scope; the latest check says: {}",
                response.summary.lines().next().unwrap_or_default()
            ),
            recommendation: "Review the failing rule and the change together: fix the approach, or adjust the rule through wh change.".into(),
            trigger: HandTrigger::SecondFailedRepair,
            rule: key.split(',').next().filter(|rule| rule.contains('.')).map(str::to_owned),
        };
        let raised = service.execute(ServiceRequest::Check(CheckRequest {
            project_dir: project_dir.clone(),
            raise_hand: Some(hand),
            ..CheckRequest::default()
        }));
        let Some(issue) = raised.data["issue"].as_str().map(str::to_owned) else {
            // Nothing was filed: say so, and leave the hand unset so a later
            // stop (after bd init, say) files it.
            write_state(&json!({"key": key, "returns": returns}));
            return emit(
                false,
                &format!(
                    "{text}\nWhetstone: two repairs did not clear these failures, and no hand could be raised ({}). Stop repairing, do not weaken the rule, and ask the owner directly.",
                    raised.summary
                ),
            );
        };
        write_state(&json!({"key": key, "returns": returns, "hand": issue}));
        return emit(
            false,
            &format!(
                "{text}\nWhetstone: two repairs did not clear these failures. A hand was raised for the owner ({issue}: {}). Stop repairing, do not weaken the rule, and take other ready work (bd ready).",
                raised.summary
            ),
        );
    }
    if returns >= 3 {
        write_state(&json!({"key": key, "returns": returns, "hand": previous["hand"]}));
        return emit(
            false,
            &format!("{text}\nWhetstone: a hand is already raised for these failures ({}); take other ready work.", previous["hand"].as_str().unwrap_or("filed")),
        );
    }
    write_state(&json!({"key": key, "returns": returns}));
    emit(true, &text)
}

/// A Git hook: print the findings people and agents need, and block the
/// commit or push unless the must rules hold.
pub(super) fn run_git_hook(
    service: &CommandService,
    request: CheckRequest,
    pre_commit: bool,
) -> i32 {
    let started = Instant::now();
    let response = service.execute(ServiceRequest::Check(request));
    let elapsed = started.elapsed();
    let label = if pre_commit { "pre-commit" } else { "pre-push" };
    let rendered = format_human_response(&response);
    match response.state {
        ServiceState::Success
            if response.data["gates"]
                .as_array()
                .map_or(true, Vec::is_empty) =>
        {
            // Nothing ran here (for example no rule runs at pre-commit): say
            // why rather than claiming zero rules hold.
            eprintln!(
                "whetstone {label}: {} ({:.1}s)",
                response.summary.lines().next().unwrap_or_default(),
                elapsed.as_secs_f64()
            );
            0
        }
        ServiceState::Success => {
            let gates = response.data["gates"].as_array().map_or(0, Vec::len);
            let flags = response.data["flags"].as_array().map_or(0, Vec::len);
            eprintln!(
                "whetstone {label}: {gates} rule(s) hold{} ({:.1}s)",
                if flags > 0 {
                    format!("; {flags} should-rule flag(s) to repair or have the owner label")
                } else {
                    String::new()
                },
                elapsed.as_secs_f64()
            );
            0
        }
        _ => {
            eprint!("{rendered}");
            eprintln!(
                "whetstone {label}: blocked ({:?}). Repair and try again; never bypass the hook with --no-verify.",
                response.state
            );
            1
        }
    }
}
