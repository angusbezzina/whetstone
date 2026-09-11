//! Agent-host integration: the same canonical skill projected into every
//! host, plus the host-specific way feedback reaches the working agent.
//!
//! Claude Code is hook-capable: a `SessionStart` hook prints the current
//! context (or says the skill is stale and must not be relied on), and a
//! `Stop` hook runs `wh check --changed` and hands violations back to the
//! same session (exit code 2 feeds stderr to the model). Cursor uses explicit
//! checkpoints: the skill tells the agent to run
//! `wh check --changed --host cursor` before handing off.
//!
//! Delivery is never assumed. A checkpoint records which skill revision the
//! host actually has on disk; a host with no checkpoint is "not
//! acknowledged", and a projection older than the accepted records is stale
//! and reported, not served.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::projection::SkillManifest;
use crate::storage::ProjectLayout;

pub const CLAUDE_SETTINGS: &str = ".claude/settings.json";
pub const HOST_SUBJECT_PREFIX: &str = "host:";
/// Bumped when the hook contract changes; a change triggers the regression set.
pub const ADAPTER_VERSION: &str = "whetstone-hooks-v1";

/// How one host's copy of the skill compares to the manifest and the
/// accepted agreement.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HostProjection {
    pub host: String,
    pub directory: String,
    /// current | stale | modified | missing
    pub state: &'static str,
    pub detail: String,
    pub delivery: &'static str,
    pub skill_digest: Option<String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn delivery(host: &str) -> &'static str {
    match host {
        "claude" => "hooks: SessionStart context and Stop-time repair feedback",
        "cursor" => "explicit checkpoint: wh check --changed --host cursor before handoff",
        _ => "explicit checkpoint: wh check --changed --host <name> before handoff",
    }
}

/// Compare each host's files with the manifest and the in-force digest.
pub fn projections(
    project_root: &Path,
    manifest: Option<&SkillManifest>,
    in_force_digest: &str,
) -> Vec<HostProjection> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for host in &manifest.hosts {
        let Some((_, directory)) = crate::skill::HOSTS.iter().find(|(name, _)| name == host) else {
            continue;
        };
        let files = manifest
            .files
            .iter()
            .filter(|file| file.path.starts_with(directory))
            .collect::<Vec<_>>();
        let mut missing = 0;
        let mut modified = 0;
        for file in &files {
            match fs::read(project_root.join(&file.path)) {
                Ok(bytes) if digest(&bytes) == file.digest => {}
                Ok(_) => modified += 1,
                Err(_) => missing += 1,
            }
        }
        let skill_digest = files
            .iter()
            .find(|file| file.path.ends_with("/SKILL.md"))
            .map(|file| file.digest.clone());
        let (state, detail) = if missing > 0 {
            ("missing", format!("{missing} generated file(s) are missing; regenerate with wh init --action wire."))
        } else if modified > 0 {
            ("modified", format!("{modified} file(s) were edited by hand; return the edits with wh init --action import, or regenerate."))
        } else if manifest.agreement_digest != in_force_digest {
            ("stale", "The accepted agreement changed since this projection was rendered; it must not be relied on until regenerated with wh init --action wire.".to_string())
        } else {
            ("current", "Matches the accepted agreement.".to_string())
        };
        result.push(HostProjection {
            host: host.clone(),
            directory: (*directory).to_string(),
            state,
            detail,
            delivery: delivery(host),
            skill_digest,
        });
    }
    result
}

/// The SessionStart context: short, canonical, and never pointing at a stale
/// projection.
pub fn session_context(dash: &Value, host: &str) -> String {
    let current = &dash["data"]["current"];
    if current["established"] != true {
        return "Whetstone: this project has no accepted agreement yet; `wh init` records it. No project policy applies until then.".into();
    }
    let hosts = current["skill"]["projections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mine = hosts.iter().find(|projection| projection["host"] == host);
    let mission = current["mission"]["title"]
        .as_str()
        .unwrap_or("(no mission)");
    let mut lines = vec![format!("Whetstone project context. Mission: {mission}")];
    match mine.and_then(|projection| projection["state"].as_str()) {
        Some("current") => lines.push(format!(
            "The verify skill in {} is current: read it before claiming a change works, and prove changes with `wh check --changed`.",
            mine.and_then(|projection| projection["directory"].as_str()).unwrap_or("the skill directory")
        )),
        Some(state) => lines.push(format!(
            "The verify skill for this host is {state}: {} Do not rely on it until `wh init --action wire` regenerates it; use `wh dash --json` for the accepted records.",
            mine.and_then(|projection| projection["detail"].as_str()).unwrap_or("")
        )),
        None => lines.push("No verify skill is installed for this host; `wh init --action wire --host <host>` generates it. Use `wh dash --json` for the accepted records.".into()),
    }
    let gates = current["checks"]["gates"]
        .as_array()
        .map(|gates| {
            gates
                .iter()
                .filter(|gate| gate["eligible"] == true)
                .filter_map(|gate| gate["id"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if !gates.is_empty() {
        lines.push(format!("Gates in force (never weaken them): {gates}."));
    }
    if let Some(first) = current["attention"]
        .as_array()
        .and_then(|items| items.first())
    {
        lines.push(format!(
            "First attention item: {} — {}",
            first["title"].as_str().unwrap_or_default(),
            first["next"].as_str().unwrap_or_default()
        ));
    }
    lines.join("\n")
}

/// Stop-hook feedback: a violation goes back to the same session (exit 2 with
/// the repair brief on stderr); anything else lets the agent stop, saying
/// plainly when the result is not a pass.
pub fn stop_feedback(check: &Value) -> (i32, String) {
    match check["state"].as_str().unwrap_or("unknown") {
        "violated" => {
            let mut lines = vec![format!(
                "Whetstone: required checks failed on this change. {}",
                check["summary"].as_str().unwrap_or_default()
            )];
            for gate in check["data"]["gates"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                if gate["state"] == "fail" {
                    lines.push(format!(
                        "- {}: {}",
                        gate["id"].as_str().unwrap_or_default(),
                        gate["summary"].as_str().unwrap_or_default()
                    ));
                    for failure in gate["failures"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .take(5)
                    {
                        lines.push(format!(
                            "  {} {}",
                            failure["location"].as_str().unwrap_or_default(),
                            failure["message"].as_str().unwrap_or_default()
                        ));
                    }
                }
            }
            for finding in check["data"]["report"]["findings"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .take(5)
            {
                lines.push(format!(
                    "- {}:{} {}",
                    finding["file"].as_str().unwrap_or("?"),
                    finding["line"].as_u64().unwrap_or(0),
                    finding["observed"].as_str().unwrap_or_default()
                ));
            }
            lines.push("Repair within the current task, never weaken a gate, then stop again to recheck. If the fix needs a policy change, stop and hand back to the owner.".into());
            (2, lines.join("\n"))
        }
        "success" => (0, String::new()),
        other => (
            0,
            format!(
                "Whetstone: the change-scoped check is {other}, not a pass: {}",
                check["summary"].as_str().unwrap_or_default()
            ),
        ),
    }
}

/// The hook entries Whetstone owns in `.claude/settings.json`, merged into
/// whatever the team already has; other hooks are left untouched.
pub fn merged_claude_settings(existing: Option<&str>) -> Result<(String, bool), String> {
    let mut settings: Value = match existing {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text)
            .map_err(|error| format!("{CLAUDE_SETTINGS} is not valid JSON: {error}"))?,
        _ => json!({}),
    };
    let before = settings.clone();
    let hooks = settings
        .as_object_mut()
        .ok_or_else(|| format!("{CLAUDE_SETTINGS} must be a JSON object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| format!("{CLAUDE_SETTINGS} hooks must be an object"))?;
    for (event, command) in [
        ("SessionStart", "wh dash --hook session-start --host claude"),
        ("Stop", "wh check --hook stop --host claude"),
    ] {
        let entries = hooks.entry(event).or_insert_with(|| json!([]));
        let Some(entries) = entries.as_array_mut() else {
            return Err(format!("{CLAUDE_SETTINGS} hooks.{event} must be an array"));
        };
        let present = entries.iter().any(|entry| {
            entry["hooks"]
                .as_array()
                .is_some_and(|hooks| hooks.iter().any(|hook| hook["command"] == command))
        });
        if !present {
            entries.push(json!({
                "hooks": [{"type": "command", "command": command, "timeout": 600}]
            }));
        }
    }
    let changed = settings != before;
    let text = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())? + "\n";
    Ok((text, changed))
}

/// The skill digest a host has on disk right now, for acknowledgement.
pub fn installed_skill_digest(layout: &ProjectLayout, host: &str) -> Option<String> {
    let (_, directory) = crate::skill::HOSTS.iter().find(|(name, _)| *name == host)?;
    let root = layout.project_root().join(directory);
    let entry = fs::read_dir(&root)
        .ok()?
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with("verify-"))?;
    fs::read(entry.path().join("SKILL.md"))
        .ok()
        .map(|bytes| digest(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_settings_merge_keeps_existing_hooks_and_is_idempotent() {
        let existing = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"make lint"}]}]}}"#;
        let (merged, changed) = merged_claude_settings(Some(existing)).expect("merge");
        assert!(changed);
        let value: Value = serde_json::from_str(&merged).expect("json");
        assert_eq!(value["model"], "opus");
        let stop = value["hooks"]["Stop"].as_array().expect("stop");
        assert_eq!(stop.len(), 2, "the team's own hook stays");
        let (again, changed) = merged_claude_settings(Some(&merged)).expect("merge");
        assert!(!changed);
        assert_eq!(again, merged);
    }

    #[test]
    fn stop_feedback_returns_violations_to_the_session_and_never_calls_unknown_a_pass() {
        let violated = json!({"state": "violated", "summary": "1 gate failed", "data": {"gates": [{"id": "standard.x", "state": "fail", "summary": "Failed", "failures": [{"location": "src/a.rs:3", "message": "boom"}]}]}});
        let (code, text) = stop_feedback(&violated);
        assert_eq!(code, 2);
        assert!(text.contains("src/a.rs:3 boom"));
        assert!(text.contains("never weaken a gate"));
        let (code, text) = stop_feedback(&json!({"state": "unknown", "summary": "nothing ran"}));
        assert_eq!(code, 0);
        assert!(text.contains("not a pass"));
        assert_eq!(
            stop_feedback(&json!({"state": "success"})),
            (0, String::new())
        );
    }

    #[test]
    fn session_context_refuses_to_point_at_a_stale_skill() {
        let dash = json!({"data": {"current": {
            "established": true,
            "mission": {"title": "Ship trust"},
            "skill": {"projections": [{"host": "claude", "state": "stale", "directory": ".claude/skills", "detail": "The accepted agreement changed."}]},
            "checks": {"gates": [{"id": "standard.x", "eligible": true}]},
            "attention": []
        }}});
        let text = session_context(&dash, "claude");
        assert!(text.contains("is stale"));
        assert!(text.contains("Do not rely on it"));
        assert!(!text.contains("is current"));
        assert!(text.contains("standard.x"));
    }
}
