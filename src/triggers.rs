//! Advisory automation hooks — session start, post-merge git, and a scheduled CI workflow.
//!
//! Every trigger is deliberately advisory: nothing blocks a merge, nothing
//! auto-extracts, nothing phones home. The generated files surface freshness
//! information and let the user decide whether to act.

use anyhow::{anyhow, Result};
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
            Ok(path) => wrote.push(json!({
                "kind": "git-hook",
                "name": "post-merge",
                "path": path.display().to_string(),
            })),
            Err(e) => warnings.push(format!("post-merge hook: {e}")),
        }
    }

    if opts.session {
        match install_session_hooks(project_dir) {
            Ok(paths) => {
                for p in paths {
                    wrote.push(json!({
                        "kind": "session-hook",
                        "path": p.display().to_string(),
                    }));
                }
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

fn install_post_merge_hook(project_dir: &Path) -> Result<PathBuf> {
    let hooks_dir = project_dir.join(".githooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let path = hooks_dir.join("post-merge");
    std::fs::write(&path, POST_MERGE_HOOK_BODY)?;
    set_executable(&path)?;

    // Wire core.hooksPath so `git pull` actually runs the hook. We avoid
    // overwriting an existing value that may intentionally point elsewhere.
    if project_dir.join(".git").exists() {
        if let Ok(current) = Command::new("git")
            .args(["config", "--get", "core.hooksPath"])
            .current_dir(project_dir)
            .output()
        {
            let existing = String::from_utf8_lossy(&current.stdout).trim().to_string();
            if existing.is_empty() {
                let status = Command::new("git")
                    .args(["config", "core.hooksPath", ".githooks"])
                    .current_dir(project_dir)
                    .status()
                    .map_err(|e| anyhow!("git config failed: {e}"))?;
                if !status.success() {
                    return Err(anyhow!("git config core.hooksPath returned non-zero"));
                }
            }
        }
    }

    Ok(path)
}

/// Claude Code + Cursor both look for project-level settings at known paths.
/// We install a minimal config that runs `wh status` advisorially on startup.
fn install_session_hooks(project_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    // Claude Code hook
    let claude_dir = project_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir)?;
    let claude_path = claude_dir.join("whetstone-session-hook.sh");
    std::fs::write(&claude_path, SESSION_HOOK_BODY)?;
    set_executable(&claude_path)?;
    written.push(claude_path.clone());

    // settings.json merges into any existing file so user-configured hooks
    // survive. `atomic_write` guards against mid-write crashes corrupting the
    // user's Claude Code config.
    let settings_path = claude_dir.join("settings.json");
    let merged = merge_claude_settings(&settings_path, &claude_path);
    crate::state::atomic_write(&settings_path, &merged);
    written.push(settings_path);

    // Cursor settings live at .cursor/config.json. We write a small advisory
    // file pointing at the same shell script; Cursor doesn't standardise
    // startup hooks, so this is documentary rather than mechanical.
    let cursor_dir = project_dir.join(".cursor");
    std::fs::create_dir_all(&cursor_dir)?;
    let cursor_path = cursor_dir.join("whetstone-session.md");
    std::fs::write(
        &cursor_path,
        "# Whetstone session advisory\n\n\
         On project open, run `wh status --json --no-snapshot --no-drift-check` and surface\n\
         a brief summary if the score is below 80 or drift is pending.\n",
    )?;
    written.push(cursor_path);

    Ok(written)
}

fn merge_claude_settings(path: &Path, hook_script: &Path) -> Value {
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
    let session_list = hooks_obj
        .entry("SessionStart".to_string())
        .or_insert_with(|| json!([]));

    let advisory = json!({
        "type": "command",
        "command": hook_script.display().to_string(),
        "description": "Whetstone freshness advisory (read-only status check)."
    });

    if let Some(arr) = session_list.as_array_mut() {
        let already = arr.iter().any(|entry| {
            entry
                .get("command")
                .and_then(|v| v.as_str())
                .map(|cmd| cmd == hook_script.display().to_string())
                .unwrap_or(false)
        });
        if !already {
            arr.push(advisory);
        }
    } else {
        *session_list = json!([advisory]);
    }

    root
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
        r##"name: Whetstone freshness check

# Auto-generated by `wh init --ci --schedule={schedule}`.
# Edit the cron expression below if you want a different cadence, or rerun the
# command with a different --schedule. Advisory only — does not block PRs.

on:
  schedule:
    - cron: "{cron}"
  workflow_dispatch: {{}}

permissions:
  contents: read
  issues: write

jobs:
  freshness:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Whetstone
        run: curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh

      - name: Run wh status
        id: status
        run: |
          set -euo pipefail
          wh status --json --no-snapshot > status.json
          cat status.json
          echo "score=$(jq -r '.score // 0' status.json)" >> "$GITHUB_OUTPUT"
          echo "label=$(jq -r '.label // \"Unknown\"' status.json)" >> "$GITHUB_OUTPUT"

      - name: Summarize to GitHub step summary
        run: |
          {{
            echo "# Whetstone freshness check"
            echo ""
            echo "- Score: ${{{{ steps.status.outputs.score }}}}"
            echo "- Label: ${{{{ steps.status.outputs.label }}}}"
            echo ""
            echo "Full status payload:"
            echo ""
            echo '```json'
            cat status.json
            echo '```'
          }} >> "$GITHUB_STEP_SUMMARY"

      - name: Run wh ci freshness gate
        run: wh ci --json --fail-on=stale
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
