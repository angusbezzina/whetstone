//! Agent-host and Git integration: the same canonical skill projected into
//! every host, the host-specific way feedback reaches the working agent, and
//! the Git hooks every tool passes through.
//!
//! The gates are Git's: `pre-commit` runs `wh check --staged` (mechanical
//! content checks on the index) and `pre-push` runs the full `wh check` for
//! the commits being pushed. An existing hook is chained, never replaced.
//!
//! Agent hooks are the fast repair loop. Claude Code, Codex and Cursor each
//! get a session-start hook (the canonical context, or a warning that the
//! skill is stale) and a stop hook that runs the change-scoped check and
//! returns violations to the same session, bounded: the same failing rules
//! after two repairs raise a hand instead of a third attempt. Each host's
//! hook speaks that host's contract (Claude: exit 2 and stderr; Codex: a
//! `decision: block` JSON; Cursor: a `followup_message` JSON).
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
pub const CODEX_HOOKS: &str = ".codex/hooks.json";
pub const CURSOR_HOOKS: &str = ".cursor/hooks.json";
/// The first line of every Git hook Whetstone writes.
pub const GIT_HOOK_MARKER: &str = "# whetstone-hook v1";
/// Where an earlier hook is kept and chained from.
pub const CHAINED_SUFFIX: &str = ".pre-whetstone";
pub const HOST_SUBJECT_PREFIX: &str = "host:";
/// Bumped when the hook contract changes; a change triggers the regression set.
pub const ADAPTER_VERSION: &str = "whetstone-hooks-v2";

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
        "claude" => "hooks: SessionStart context and Stop-time repair feedback (.claude/settings.json)",
        "cursor" => "hooks: sessionStart context and stop follow-ups (.cursor/hooks.json)",
        "codex" => "hooks: SessionStart context and Stop continuation (.codex/hooks.json; approve them in /hooks)",
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
    if let Some(regression) = current["skill"]["acknowledged"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["host"] == host))
        .and_then(|row| row["regression"].as_str())
    {
        lines.push(regression.to_string());
    }
    let rules = current["rules"]
        .as_array()
        .map(|rules| {
            rules
                .iter()
                .filter(|rule| rule["lifecycle"] == "accepted")
                .map(|rule| {
                    format!(
                        "{} [{} · {}{}]",
                        rule["title"].as_str().unwrap_or_default(),
                        rule["strength"].as_str().unwrap_or_default(),
                        rule["enforcer"].as_str().unwrap_or_default(),
                        if rule["shadow"] == true {
                            " · shadow"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !rules.is_empty() {
        lines.push(format!(
            "Rules in force (never weaken them): {}.",
            rules.join("; ")
        ));
    }
    lines.push("Before building: brief yourself with pstack /how, /why and /blast-radius and record it (wh check --brief). Raise a hand (wh check --raise-hand) instead of guessing when a must rule is unavailable, a finding survives a second repair, the change touches an owner-reserved area, or the spec is vague.".into());
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
            lines.push("Repair within the current task, never weaken a rule, then stop again to recheck. If the fix needs a rule change or the owner's call, raise a hand (wh check --raise-hand) and take other ready work.".into());
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

/// Codex hooks (`.codex/hooks.json`), merged into what the team has. Codex
/// runs a project's hooks only after the user trusts them in `/hooks`.
pub fn merged_codex_hooks(existing: Option<&str>) -> Result<(String, bool), String> {
    merged_event_hooks(
        existing,
        CODEX_HOOKS,
        &[
            (
                "SessionStart",
                "wh dash --hook session-start --host codex",
                30,
            ),
            ("Stop", "wh check --hook stop --host codex", 600),
        ],
        |command, timeout| json!({"hooks": [{"type": "command", "command": command, "timeout": timeout}]}),
        |entry, command| {
            entry["hooks"]
                .as_array()
                .is_some_and(|hooks| hooks.iter().any(|hook| hook["command"] == command))
        },
    )
}

/// Cursor hooks (`.cursor/hooks.json`, version 1), merged into what the team
/// has. The stop hook's own loop limit is set to Whetstone's bound.
pub fn merged_cursor_hooks(existing: Option<&str>) -> Result<(String, bool), String> {
    let (text, changed) = merged_event_hooks(
        existing,
        CURSOR_HOOKS,
        &[
            (
                "sessionStart",
                "wh dash --hook session-start --host cursor",
                30,
            ),
            ("stop", "wh check --hook stop --host cursor", 600),
        ],
        |command, _| {
            if command.contains("--hook stop") {
                json!({"command": command, "loop_limit": 3})
            } else {
                json!({"command": command})
            }
        },
        |entry, command| entry["command"] == command,
    )?;
    let mut value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let mut changed = changed;
    if value.get("version").is_none() {
        value["version"] = json!(1);
        changed = true;
    }
    Ok((
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())? + "\n",
        changed,
    ))
}

fn merged_event_hooks(
    existing: Option<&str>,
    name: &str,
    events: &[(&str, &str, u64)],
    entry: impl Fn(&str, u64) -> Value,
    present: impl Fn(&Value, &str) -> bool,
) -> Result<(String, bool), String> {
    let mut settings: Value = match existing {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text)
            .map_err(|error| format!("{name} is not valid JSON: {error}"))?,
        _ => json!({}),
    };
    let before = settings.clone();
    let hooks = settings
        .as_object_mut()
        .ok_or_else(|| format!("{name} must be a JSON object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| format!("{name} hooks must be an object"))?;
    for (event, command, timeout) in events {
        let entries = hooks.entry(*event).or_insert_with(|| json!([]));
        let Some(entries) = entries.as_array_mut() else {
            return Err(format!("{name} hooks.{event} must be an array"));
        };
        if !entries.iter().any(|existing| present(existing, command)) {
            entries.push(entry(command, *timeout));
        }
    }
    let changed = settings != before;
    let text = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())? + "\n";
    Ok((text, changed))
}

/// Whether the Git gates are installed, and where the hooks live.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct GitHooks {
    pub directory: Option<String>,
    pub pre_commit: bool,
    pub pre_push: bool,
    /// Earlier hooks Whetstone chains to.
    pub chained: Vec<String>,
}

/// The repository's hooks directory (respecting `core.hooksPath`).
pub fn git_hooks_dir(project_root: &Path) -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

pub fn git_hooks(project_root: &Path) -> GitHooks {
    let Some(directory) = git_hooks_dir(project_root) else {
        return GitHooks::default();
    };
    let ours = |name: &str| {
        fs::read_to_string(directory.join(name))
            .is_ok_and(|text| text.lines().nth(1) == Some(GIT_HOOK_MARKER))
    };
    let chained = ["pre-commit", "pre-push"]
        .into_iter()
        .filter(|name| directory.join(format!("{name}{CHAINED_SUFFIX}")).is_file())
        .map(|name| format!("{name}{CHAINED_SUFFIX}"))
        .collect();
    GitHooks {
        directory: Some(
            directory
                .strip_prefix(project_root)
                .unwrap_or(&directory)
                .display()
                .to_string(),
        ),
        pre_commit: ours("pre-commit"),
        pre_push: ours("pre-push"),
        chained,
    }
}

const WH_LOOKUP: &str = r#"if command -v wh >/dev/null 2>&1; then WH=wh
elif command -v whetstone >/dev/null 2>&1; then WH=whetstone
else
  echo "whetstone: wh is not installed, so the rules did not run. Install it, or remove this hook deliberately." >&2
  exit 1
fi"#;

/// The pre-commit hook: fast mechanical checks on the staged index.
pub fn pre_commit_hook() -> String {
    format!(
        r#"#!/bin/sh
{GIT_HOOK_MARKER}
# Installed by `wh init --action wire --hooks`. Runs the rules' fast
# mechanical checks (AST, design tokens, public surface) on the staged index
# only, then any hook that was here before (kept as pre-commit{CHAINED_SUFFIX}).
hook_dir=$(dirname "$0")
if [ -x "$hook_dir/pre-commit{CHAINED_SUFFIX}" ]; then
  "$hook_dir/pre-commit{CHAINED_SUFFIX}" "$@" || exit $?
fi
{WH_LOOKUP}
exec "$WH" check --staged --hook pre-commit
"#
    )
}

/// The pre-push hook: the full check (mechanical, Jev, reviews, fresh
/// proofs) for the commits being pushed, after any earlier hook.
pub fn pre_push_hook() -> String {
    format!(
        r#"#!/bin/sh
{GIT_HOOK_MARKER}
# Installed by `wh init --action wire --hooks`. Runs any hook that was here
# before (kept as pre-push{CHAINED_SUFFIX}), then the full `wh check` for the
# commits being pushed: mechanical rules, Jev questions (unless shadow or
# local-only), review attestations and fresh proofs.
hook_dir=$(dirname "$0")
refs=$(cat)
if [ -x "$hook_dir/pre-push{CHAINED_SUFFIX}" ]; then
  printf '%s
' "$refs" | "$hook_dir/pre-push{CHAINED_SUFFIX}" "$@" || exit $?
fi
{WH_LOOKUP}
zero=0000000000000000000000000000000000000000
status=0
printf '%s
' "$refs" | {{
  while read -r local_ref local_sha remote_ref remote_sha; do
    [ -z "$local_sha" ] && continue
    [ "$local_sha" = "$zero" ] && continue
    if [ "$remote_sha" = "$zero" ] || [ -z "$remote_sha" ]; then
      # A new branch: compare with the remote's default branch. Git passes
      # the remote's name as $1 and its URL as $2.
      base=
      remote_head=$(git symbolic-ref -q "refs/remotes/$1/HEAD" 2>/dev/null || true)
      for candidate in "$remote_head" "$1/main" "$1/master"; do
        if [ -z "$base" ] && [ -n "$candidate" ]; then
          base=$(git merge-base "$local_sha" "$candidate" 2>/dev/null || true)
        fi
      done
    else
      base=$remote_sha
    fi
    if [ -n "$base" ]; then
      "$WH" check --base "$base" --pushed "$local_sha" --hook pre-push || exit 1
    else
      "$WH" check --pushed "$local_sha" --hook pre-push || exit 1
    fi
  done
}} || status=1
exit $status
"#
    )
}

/// One planned hook write: the file, its content, and the earlier hook it
/// moves aside to chain.
#[derive(Debug, Clone, Serialize)]
pub struct HookWrite {
    pub path: String,
    pub bytes: usize,
    pub chains: Option<String>,
    #[serde(skip)]
    pub contents: String,
}

/// The writes that install both Git gates; nothing when already installed.
pub fn git_hook_plan(project_root: &Path) -> Result<Vec<HookWrite>, String> {
    let directory = git_hooks_dir(project_root)
        .ok_or("This is not a Git repository, so there is nowhere to install the gates.")?;
    let mut plan = Vec::new();
    for (name, contents) in [
        ("pre-commit", pre_commit_hook()),
        ("pre-push", pre_push_hook()),
    ] {
        let path = directory.join(name);
        let existing = fs::read_to_string(&path).ok();
        if existing.as_deref() == Some(contents.as_str()) {
            continue;
        }
        let chains = match &existing {
            Some(text) if text.lines().nth(1) != Some(GIT_HOOK_MARKER) => {
                if directory.join(format!("{name}{CHAINED_SUFFIX}")).exists() {
                    return Err(format!(
                        "{} and {name}{CHAINED_SUFFIX} both exist; merge them by hand before installing the Whetstone gate.",
                        path.display()
                    ));
                }
                Some(format!("{name}{CHAINED_SUFFIX}"))
            }
            _ => None,
        };
        plan.push(HookWrite {
            path: path
                .strip_prefix(project_root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            bytes: contents.len(),
            chains,
            contents,
        });
    }
    Ok(plan)
}

/// Apply a hook plan: move each earlier hook aside, then write ours.
pub fn install_git_hooks(project_root: &Path, plan: &[HookWrite]) -> Result<(), String> {
    for write in plan {
        let path = project_root.join(&write.path);
        let path = path.as_path();
        if let (Some(chained), Some(directory)) = (&write.chains, path.parent()) {
            fs::rename(path, directory.join(chained)).map_err(|error| error.to_string())?;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, &write.contents).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// The session-start output in each host's format.
pub fn session_output(host: &str, context: &str) -> String {
    match host {
        "cursor" => json!({"additional_context": context}).to_string(),
        "codex" => json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}).to_string(),
        _ => context.to_string(),
    }
}

/// How a stop hook hands feedback back in each host's format:
/// `(exit code, stdout, stderr)`.
pub fn stop_output(host: &str, block: bool, text: &str) -> (i32, String, String) {
    match (host, block) {
        ("cursor", true) => (
            0,
            json!({"followup_message": text}).to_string(),
            String::new(),
        ),
        ("cursor", false) => (0, "{}".into(), text.to_string()),
        ("codex", true) => (
            0,
            json!({"decision": "block", "reason": text}).to_string(),
            String::new(),
        ),
        ("codex", false) => (0, "{}".into(), text.to_string()),
        (_, true) => (2, String::new(), text.to_string()),
        (_, false) => (0, String::new(), text.to_string()),
    }
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
    fn codex_and_cursor_hooks_merge_idempotently_in_their_own_formats() {
        let (codex, changed) = merged_codex_hooks(None).expect("codex");
        assert!(changed);
        let value: Value = serde_json::from_str(&codex).expect("json");
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"][0]["command"],
            "wh check --hook stop --host codex"
        );
        assert!(!merged_codex_hooks(Some(&codex)).expect("again").1);
        let (cursor, _) =
            merged_cursor_hooks(Some(r#"{"hooks":{"stop":[{"command":"./lint.sh"}]}}"#))
                .expect("cursor");
        let value: Value = serde_json::from_str(&cursor).expect("json");
        assert_eq!(value["version"], 1);
        assert_eq!(value["hooks"]["stop"].as_array().expect("stop").len(), 2);
        assert_eq!(value["hooks"]["stop"][1]["loop_limit"], 3);
    }

    #[test]
    fn stop_output_speaks_each_hosts_contract() {
        assert_eq!(stop_output("claude", true, "fix it").0, 2);
        let (code, stdout, _) = stop_output("codex", true, "fix it");
        assert_eq!(code, 0);
        assert_eq!(
            serde_json::from_str::<Value>(&stdout).expect("json")["decision"],
            "block"
        );
        let (_, stdout, _) = stop_output("cursor", true, "fix it");
        assert_eq!(
            serde_json::from_str::<Value>(&stdout).expect("json")["followup_message"],
            "fix it"
        );
        assert_eq!(stop_output("codex", false, "").1, "{}");
    }

    #[test]
    fn git_hooks_chain_an_existing_hook_and_are_idempotent() {
        let temp = tempfile::tempdir().expect("temp");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .expect("git");
        let hooks = git_hooks_dir(temp.path()).expect("dir");
        fs::create_dir_all(&hooks).expect("hooks");
        fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("existing");
        let plan = git_hook_plan(temp.path()).expect("plan");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].chains.as_deref(), Some("pre-commit.pre-whetstone"));
        install_git_hooks(temp.path(), &plan).expect("install");
        let state = git_hooks(temp.path());
        assert!(state.pre_commit && state.pre_push);
        assert_eq!(state.chained, vec!["pre-commit.pre-whetstone".to_string()]);
        assert!(git_hook_plan(temp.path()).expect("again").is_empty());
    }

    #[test]
    fn stop_feedback_returns_violations_to_the_session_and_never_calls_unknown_a_pass() {
        let violated = json!({"state": "violated", "summary": "1 gate failed", "data": {"gates": [{"id": "standard.x", "state": "fail", "summary": "Failed", "failures": [{"location": "src/a.rs:3", "message": "boom"}]}]}});
        let (code, text) = stop_feedback(&violated);
        assert_eq!(code, 2);
        assert!(text.contains("src/a.rs:3 boom"));
        assert!(text.contains("never weaken a rule"));
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
            "rules": [{"title": "Tests pass", "lifecycle": "accepted", "strength": "must", "enforcer": "Test"}],
            "attention": []
        }}});
        let text = session_context(&dash, "claude");
        assert!(text.contains("is stale"));
        assert!(text.contains("Do not rely on it"));
        assert!(!text.contains("is current"));
        assert!(text.contains("Tests pass [must · Test]"));
    }
}
