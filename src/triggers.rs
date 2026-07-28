//! Advisory automation hooks — session start, post-merge git, and a scheduled CI workflow.
//!
//! Every trigger is deliberately advisory: nothing blocks a merge, nothing
//! auto-extracts, nothing phones home. The generated files surface freshness
//! information and let the user decide whether to act.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `wh init --hooks` should install. The CLI wires a default of `"all"`.
pub struct HookOptions {
    pub session: bool,
    pub post_merge: bool,
}

impl HookOptions {
    pub fn all() -> Self {
        HookOptions {
            session: true,
            post_merge: true,
        }
    }
}

/// Install the local git hooks and any agent-side session configs. Returns
/// a structured JSON report of what was done.
pub fn install_hooks(project_dir: &Path, opts: &HookOptions) -> Result<Value> {
    let mut wrote: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if opts.post_merge {
        match install_post_merge_hook(project_dir) {
            Ok(path) => {
                wrote.push(json!({
                    "kind": "git-hook",
                    "name": "post-merge",
                    "path": path.display().to_string(),
                }));
                match wire_hooks_path(project_dir) {
                    Ok(Some(reason)) => warnings.push(format!("post-merge hook: {reason}")),
                    Ok(None) => {}
                    Err(e) => warnings.push(format!("post-merge hook: {e}")),
                }
            }
            Err(e) => warnings.push(format!("post-merge hook: {e}")),
        }
    }

    if opts.session {
        match install_session_hooks(project_dir) {
            Ok(outcome) => {
                for p in outcome.wrote {
                    wrote.push(json!({
                        "kind": "session-hook",
                        "path": p.display().to_string(),
                    }));
                }
                warnings.extend(outcome.skipped);
            }
            Err(e) => warnings.push(format!("session hook: {e}")),
        }
    }

    Ok(json!({
        "status": "ok",
        "wrote": wrote,
        "warnings": warnings,
        "next_command": "Review the generated files, then commit the ones you want checked in.",
    }))
}

/// Write `.github/workflows/whetstone-check.yml`.
pub fn install_ci_workflow(project_dir: &Path, schedule: &str) -> Result<Value> {
    let cron = schedule_to_cron(schedule)?;
    let path = project_dir
        .join(".github")
        .join("workflows")
        .join("whetstone-check.yml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let body = render_workflow(&cron, schedule);
    std::fs::write(&path, body)?;

    Ok(json!({
        "status": "ok",
        "path": path.display().to_string(),
        "schedule": schedule,
        "cron": cron,
        "next_command": "Commit .github/workflows/whetstone-check.yml and push; GitHub Actions picks it up automatically.",
    }))
}

// ── git hooks ──

/// Identifies a post-merge hook Whetstone authored, across template versions.
const POST_MERGE_MARKER: &str = "Whetstone post-merge advisory";

/// True when `.git/hooks/` holds executable hooks git is actually running
/// (ignoring the `.sample` files git ships). Setting `core.hooksPath` would
/// silently disable every one of them — a `pre-commit install` / lefthook
/// layout is the common case.
pub fn has_local_git_hooks(project_dir: &Path) -> bool {
    // Resolve via git, not `project_dir/.git/hooks`: in a linked worktree
    // `.git` is a FILE, and in a monorepo package it does not exist at all —
    // in both cases a naive path check reports "no hooks" and we would redirect
    // core.hooksPath in the SHARED config, killing the user's live hooks.
    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(project_dir)
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let git_hooks = {
        let p = PathBuf::from(&raw);
        if p.is_absolute() {
            p
        } else {
            project_dir.join(p)
        }
    };
    let Ok(entries) = std::fs::read_dir(&git_hooks) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) == Some("sample") || !path.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

fn install_post_merge_hook(project_dir: &Path) -> Result<PathBuf> {
    let path = project_dir.join(".githooks").join("post-merge");
    // Private mode never modifies tracked files — exclude cannot hide the diff,
    // and overwriting a teammate's committed hook would destroy it.
    if crate::private_mode::skip_tracked(project_dir, ".githooks/post-merge") {
        return Err(anyhow!(
            ".githooks/post-merge is git-tracked and private mode never modifies tracked files — left unchanged"
        ));
    }
    // Never blind-overwrite a COMMITTED hook we did not author, in either mode:
    // this is a whole-file write, so a team's post-merge would be destroyed.
    // (`wh publish` calls this implicitly, which made it a silent side effect.)
    // Never overwrite a post-merge hook we did not author — this is a whole-file
    // write. Applies whether or not the file is tracked: an UNTRACKED hook (or
    // one under a gitignored `.githooks/`) has no committed copy to recover
    // from, so clobbering it is worse, not better. Authorship is detected by
    // MARKER rather than byte equality, so an older release's body — or a
    // teammate's tweak to ours — is still ours to update.
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.is_empty() && !existing.contains(POST_MERGE_MARKER) {
        return Err(anyhow!(
            ".githooks/post-merge already exists and was not written by Whetstone — left unchanged rather than overwriting it"
        ));
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, POST_MERGE_HOOK_BODY)?;
    set_executable(&path)?;

    Ok(path)
}

/// Wire `core.hooksPath` so `git pull` actually runs the post-merge hook, and
/// say plainly when we don't. Silence here means claiming a hook is installed
/// when it can never fire. Returns a reason when the hook is left unwired.
fn wire_hooks_path(project_dir: &Path) -> Result<Option<String>> {
    // Already pointed at our own dir by a previous run: correctly wired, and
    // nothing to warn about. Checked FIRST because `git rev-parse --git-path
    // hooks` honours core.hooksPath — so once we set it, the executable-hooks
    // probe below would find OUR post-merge hook and warn that the user has a
    // pre-commit setup we must not disturb, on every idempotent re-run.
    let current = Command::new("git")
        .args(["config", "--get", "core.hooksPath"])
        .current_dir(project_dir)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if current == ".githooks" {
        return Ok(None);
    }

    if has_local_git_hooks(project_dir) {
        return Ok(Some(
            "left core.hooksPath alone: this repo has executable hooks in its git hooks dir (a pre-commit/lefthook setup). Whetstone's post-merge advisory will not run; your existing hooks keep working.".to_string(),
        ));
    }
    let in_repo = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project_dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !in_repo {
        return Ok(Some(
            "not a git repository: the post-merge hook was written but is not wired.".to_string(),
        ));
    }
    // A package inside a monorepo: core.hooksPath is repo-wide and relative to
    // the repo root, so pointing it at this package's .githooks would hijack
    // hooks for the whole repository.
    let toplevel = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()));
    let is_root = match toplevel {
        Some(root) => {
            let root = root.canonicalize().unwrap_or(root);
            let proj = project_dir
                .canonicalize()
                .unwrap_or_else(|_| project_dir.to_path_buf());
            root == proj
        }
        None => false,
    };
    if !is_root {
        return Ok(Some(
            "left core.hooksPath alone: this project is not the git root, and core.hooksPath is repo-wide. The post-merge hook was written but is not wired.".to_string(),
        ));
    }

    if !current.is_empty() {
        return Ok(Some(format!(
            "left core.hooksPath alone: already set to `{current}`. The post-merge hook was written but is not wired."
        )));
    }
    let status = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(project_dir)
        .status()
        .with_context(|| "git config failed")?;
    if !status.success() {
        return Err(anyhow!("git config core.hooksPath returned non-zero"));
    }
    Ok(None)
}

/// Claude Code + Cursor both look for project-level settings at known paths.
/// We install a minimal config that runs `wh status` advisorially on startup.
/// What `install_session_hooks` did — and, just as importantly, what it
/// deliberately did not do. A silent skip reads as "installed".
struct SessionHookOutcome {
    wrote: Vec<PathBuf>,
    skipped: Vec<String>,
}

fn skip_note(rel: &str) -> String {
    format!(
        "{rel} is git-tracked and private mode never modifies tracked files — left unchanged (the committed copy is what will run)"
    )
}

fn install_session_hooks(project_dir: &Path) -> Result<SessionHookOutcome> {
    let mut written = Vec::new();
    let mut skipped = Vec::new();
    // Claude Code hook. Each script is skipped when private mode would
    // otherwise overwrite a tracked copy (the file already exists there, so the
    // settings entry still resolves).
    let claude_dir = project_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir)?;
    let claude_path = claude_dir.join("whetstone-session-hook.sh");
    if crate::private_mode::skip_tracked(project_dir, ".claude/whetstone-session-hook.sh") {
        skipped.push(skip_note(".claude/whetstone-session-hook.sh"));
    } else {
        std::fs::write(&claude_path, SESSION_HOOK_BODY)?;
        set_executable(&claude_path)?;
        written.push(claude_path.clone());
    }

    // In-session enforcement hook: PostToolUse scans the edited file and feeds
    // violations back to the agent in the same turn (whetstone-cpt). A tiny
    // wrapper script no-ops if `wh` is not on PATH, so a missing binary never
    // wedges the agent.
    let posttool_path = claude_dir.join("whetstone-posttooluse-hook.sh");
    if crate::private_mode::skip_tracked(project_dir, ".claude/whetstone-posttooluse-hook.sh") {
        skipped.push(skip_note(".claude/whetstone-posttooluse-hook.sh"));
    } else {
        std::fs::write(&posttool_path, POSTTOOLUSE_HOOK_BODY)?;
        set_executable(&posttool_path)?;
        written.push(posttool_path.clone());
    }

    // settings.json merges into any existing file so user-configured hooks
    // survive. `atomic_write` guards against mid-write crashes corrupting the
    // user's Claude Code config. In private mode a git-tracked settings.json is
    // never modified — hooks land in settings.local.json (Claude Code's per-user
    // overlay) instead; `wh publish` migrates them back (whetstone-xdr).
    let settings_path = if crate::private_mode::is_private(project_dir)
        && crate::private_mode::is_git_tracked(project_dir, ".claude/settings.json")
    {
        claude_dir.join("settings.local.json")
    } else {
        claude_dir.join("settings.json")
    };
    let merged = merge_claude_settings(&settings_path, &claude_path, &posttool_path);
    crate::state::atomic_write(&settings_path, &merged);
    written.push(settings_path);

    // Cursor has no standard PostToolUse hook API, so in-session enforcement is
    // not mechanical there. We write an honest advisory pointing at the CLI/MCP
    // path (Cursor reads the generated context files; run `wh scan` / use the MCP
    // server for lookups) rather than pretending parity with Claude Code.
    let cursor_dir = project_dir.join(".cursor");
    let cursor_path = cursor_dir.join("whetstone-session.md");
    if crate::private_mode::skip_tracked(project_dir, ".cursor/whetstone-session.md") {
        skipped.push(skip_note(".cursor/whetstone-session.md"));
        return Ok(SessionHookOutcome {
            wrote: written,
            skipped,
        });
    }
    std::fs::create_dir_all(&cursor_dir)?;
    std::fs::write(
        &cursor_path,
        "# Whetstone in Cursor\n\n\
         Cursor has no PostToolUse hook, so Whetstone cannot feed violations back\n\
         in-session automatically here (that is Claude Code only, for now).\n\n\
         Instead:\n\
         - The generated `.cursorrules` / `AGENTS.md` carry the rules at session start.\n\
         - Register the MCP server so the agent can look rules up mid-turn:\n\
           `wh mcp --project-dir .` (see README > Use with coding agents).\n\
         - Enforce deterministically with `wh scan .` before finishing / in CI.\n",
    )?;
    written.push(cursor_path);

    Ok(SessionHookOutcome {
        wrote: written,
        skipped,
    })
}

fn merge_claude_settings(path: &Path, session_hook: &Path, posttooluse_hook: &Path) -> Value {
    let existing = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));

    let mut root = match existing {
        Value::Object(m) => Value::Object(m),
        _ => json!({}),
    };

    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}));
    let hooks_obj = hooks.as_object_mut().unwrap();

    // SessionStart: flat advisory entry (read-only freshness check), preserved
    // in its existing shape for backward compatibility.
    let session_cmd = session_hook.display().to_string();
    let session_list = hooks_obj
        .entry("SessionStart".to_string())
        .or_insert_with(|| json!([]));
    if let Some(arr) = session_list.as_array_mut() {
        let already = arr.iter().any(|e| {
            e.get("command").and_then(|v| v.as_str()) == Some(session_cmd.as_str())
        });
        if !already {
            arr.push(json!({
                "type": "command",
                "command": session_cmd,
                "description": "Whetstone freshness advisory (read-only status check)."
            }));
        }
    } else {
        *session_list = json!([{
            "type": "command",
            "command": session_cmd,
            "description": "Whetstone freshness advisory (read-only status check)."
        }]);
    }

    // PostToolUse: matcher + hooks shape (Claude Code's standard structure), so
    // the edited file is scanned and violations fed back in-session.
    let posttool_cmd = posttooluse_hook.display().to_string();
    let posttool_list = hooks_obj
        .entry("PostToolUse".to_string())
        .or_insert_with(|| json!([]));
    if let Some(arr) = posttool_list.as_array_mut() {
        let already = arr.iter().any(|e| {
            e.get("hooks")
                .and_then(|h| h.as_array())
                .map(|hs| {
                    hs.iter()
                        .any(|x| x.get("command").and_then(|c| c.as_str()) == Some(posttool_cmd.as_str()))
                })
                .unwrap_or(false)
        });
        if !already {
            arr.push(json!({
                "matcher": "Edit|Write|MultiEdit",
                "hooks": [{ "type": "command", "command": posttool_cmd }]
            }));
        }
    } else {
        *posttool_list = json!([{
            "matcher": "Edit|Write|MultiEdit",
            "hooks": [{ "type": "command", "command": posttool_cmd }]
        }]);
    }

    root
}

/// Remove Whetstone's hook entries from `.claude/settings.local.json` — the
/// publish-time migration for hooks that private mode redirected there
/// (whetstone-xdr). User-authored entries in the file are untouched. Returns
/// true when anything was removed.
pub fn remove_whetstone_hooks_from_local(project_dir: &Path) -> Result<bool> {
    let path = project_dir.join(".claude").join("settings.local.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut doc) = serde_json::from_str::<Value>(&raw) else {
        return Ok(false);
    };
    let mut removed = false;
    let is_ours = |cmd: Option<&str>| {
        cmd.map(|c| {
            c.ends_with("whetstone-session-hook.sh") || c.ends_with("whetstone-posttooluse-hook.sh")
        })
        .unwrap_or(false)
    };
    if let Some(hooks) = doc.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        if let Some(arr) = hooks.get_mut("SessionStart").and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|e| !is_ours(e.get("command").and_then(|c| c.as_str())));
            removed |= arr.len() != before;
        }
        if let Some(arr) = hooks.get_mut("PostToolUse").and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|e| {
                !e.get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hs| hs.iter().any(|x| is_ours(x.get("command").and_then(|c| c.as_str()))))
                    .unwrap_or(false)
            });
            removed |= arr.len() != before;
        }
    }
    if removed {
        // Don't leave a hollow `{"hooks":{"PostToolUse":[],"SessionStart":[]}}`
        // behind — that would be a brand-new untracked file introduced by
        // publish. Drop empty hook lists, then the file itself if nothing of
        // the user's remains.
        if let Some(hooks) = doc.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            hooks.retain(|_, v| !v.as_array().map(|a| a.is_empty()).unwrap_or(false));
            let hooks_empty = hooks.is_empty();
            if hooks_empty {
                doc.as_object_mut().map(|o| o.remove("hooks"));
            }
        }
        if doc.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            let _ = std::fs::remove_file(&path);
        } else {
            crate::state::atomic_write(&path, &doc);
        }
    }
    Ok(removed)
}

// ── helpers ──

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn schedule_to_cron(schedule: &str) -> Result<String> {
    match schedule {
        "daily" => Ok("0 9 * * *".to_string()),
        "weekly" => Ok("0 9 * * 1".to_string()),
        // True "every other Monday" cannot be expressed in 5-field cron; the
        // closest stable approximation is the 1st and 15th of each month.
        "biweekly" => Ok("0 9 1,15 * *".to_string()),
        "monthly" => Ok("0 9 1 * *".to_string()),
        other => {
            if other.split_whitespace().count() == 5 {
                Ok(other.to_string())
            } else {
                Err(anyhow!(
                    "Unknown schedule '{other}'. Expected one of: daily, weekly, biweekly, monthly, or a 5-field cron expression."
                ))
            }
        }
    }
}

fn render_workflow(cron: &str, schedule: &str) -> String {
    // Template uses `{{` / `}}` to escape literal braces for format!(), and
    // uses a `##` raw-string delimiter so the heredoc body can carry `"#` without
    // terminating early.
    format!(
        r##"name: Whetstone

# Auto-generated by `wh init --ci --schedule={schedule}`.
#
# Two agent-free gates:
#  - enforce (push / pull_request): `wh scan` fails the build on violations of the
#    rules Whetstone's scanner enforces directly (AST rules). For rules delegated
#    to a linter (lint_proxy), run `wh actions lint` and merge the generated config
#    -- a Cargo.toml `[lints.clippy]` fragment, or a ruff/biome overlay -- into your
#    project so your normal lint CI enforces them too.
#  - freshness (schedule): `wh ci` fails on content-hash/version drift (the docs a
#    rule was derived from changed since it was authored).

on:
  push:
    branches: [main]
  pull_request:
  schedule:
    - cron: "{cron}"
  workflow_dispatch: {{}}

permissions:
  contents: read

jobs:
  enforce:
    if: github.event_name == 'push' || github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Whetstone
        run: curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh

      - name: Enforce approved rules (agent-free)
        # Fails the build on AST-rule violations. config_issues (a linter overlay
        # not yet merged) are advisory here -- run `wh actions lint` and merge them
        # so your native linters enforce the lint_proxy rules in your lint CI.
        run: |
          set -euo pipefail
          wh scan . --json --no-fail > scan.json
          jq -e '.violations_count == 0' scan.json >/dev/null \
            || {{ echo "Whetstone: rule violations found:"; wh scan .; exit 1; }}

  freshness:
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
    runs-on: ubuntu-latest
    permissions:
      contents: read
      issues: write
    steps:
      - uses: actions/checkout@v4

      - name: Install Whetstone
        run: curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh

      - name: Run wh ci freshness gate
        # Fails the run on content-hash OR version drift (needs_review or stale).
        run: wh ci --json --fail-on=needs_review
"##
    )
}

// ── file bodies ──

const POST_MERGE_HOOK_BODY: &str = r#"#!/usr/bin/env sh
# Whetstone post-merge advisory (installed by `wh init --hooks`).
# Runs after `git merge` / `git pull --rebase`; prints a one-line warning if
# dependency versions drifted since rules were last extracted. Exits 0 either
# way — does not block the merge.
set -eu

if ! command -v wh >/dev/null 2>&1; then
    exit 0
fi

drift_json="$(wh init --check-drift --changed-only --json 2>/dev/null || true)"
if [ -z "$drift_json" ]; then
    exit 0
fi

if printf '%s' "$drift_json" | grep -q '"manifests_changed":[[:space:]]*true'; then
    printf 'Whetstone: dependency drift detected after merge. Run `wh reinit` to update rules.\n' >&2
fi
exit 0
"#;

const SESSION_HOOK_BODY: &str = r#"#!/usr/bin/env sh
# Whetstone session-start advisory (installed by `wh init --hooks`).
# Claude Code / Cursor invoke this on project open. It runs `wh status` and
# surfaces a short summary when the project's rules are stale.
set -eu

if ! command -v wh >/dev/null 2>&1; then
    exit 0
fi

status_json="$(wh status --json --no-snapshot 2>/dev/null || true)"
if [ -z "$status_json" ]; then
    exit 0
fi

label="$(printf '%s' "$status_json" | awk -F'"label":"' 'NR==1 {split($2, a, "\""); print a[1]; exit}')"
score="$(printf '%s' "$status_json" | awk -F'"score":' 'NR==1 {n=$2+0; print n; exit}')"

case "$label" in
    Healthy|"")
        exit 0 ;;
    *)
        printf 'Whetstone: %s (score %s). Run `wh status` for detail.\n' "$label" "$score" >&2
        exit 0 ;;
esac
"#;

// PostToolUse hook: forwards the event JSON on stdin to `wh hook posttooluse`,
// which scans the edited file and feeds violations back to the agent in-session.
// No-ops if `wh` is not installed (fail-open at the shell level too).
const POSTTOOLUSE_HOOK_BODY: &str = r#"#!/usr/bin/env sh
# Whetstone in-session enforcement (installed by `wh init --hooks`).
# Claude Code invokes this after Edit/Write/MultiEdit with the event JSON on stdin.
set -eu

if ! command -v wh >/dev/null 2>&1; then
    exit 0
fi

exec wh hook posttooluse --project-dir "${CLAUDE_PROJECT_DIR:-.}"
"#;
