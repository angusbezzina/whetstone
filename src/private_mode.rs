//! Private mode — solo adoption with zero shared-repo footprint (whetstone-xdr).
//!
//! The beads-style model: one team member runs `wh init --claude --private` on a
//! shared repo and NOTHING shows in `git status` for teammates. Artifacts stay at
//! their normal paths; a managed block in `.git/info/exclude` (per-clone, never
//! committed) hides them. `wh publish` removes exactly that block and completes
//! any wiring that private mode skipped, making the artifacts trackable.
//!
//! Design + locked decisions: `planning/private-mode.md`. Two invariants matter
//! most: tracked files are never modified while private (exclude cannot hide
//! changes to tracked files), and enforcement (scan/hook/MCP) is mode-independent
//! — private changes visibility, never function.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const EXCLUDE_BEGIN: &str =
    "# >>> whetstone private mode (managed by `wh`; `wh publish` removes this block) >>>";
const EXCLUDE_END: &str = "# <<< whetstone private mode <<<";

/// Static: excluding an already-tracked path is a harmless no-op, and a static
/// block keeps enable/publish exactly inverse operations.
const EXCLUDE_ENTRIES: &[&str] = &[
    "/whetstone/",
    "/.mcp.json",
    "/.claude/settings.json",
    "/.claude/settings.local.json",
    "/.claude/whetstone-session-hook.sh",
    "/.claude/whetstone-posttooluse-hook.sh",
    "/.cursor/whetstone-session.md",
    "/.githooks/post-merge",
];

/// Read the `setup.private` marker. The marker lives in whetstone/whetstone.yaml,
/// which is itself excluded, so teammates never see it.
pub fn is_private(project_dir: &Path) -> bool {
    let path = project_dir.join("whetstone").join("whetstone.yaml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_yaml::from_str::<Value>(&s).ok())
        .and_then(|d| {
            d.get("setup")
                .and_then(|s| s.get("private"))
                .and_then(|p| p.as_bool())
        })
        .unwrap_or(false)
}

/// True if git tracks anything matching `rel` (a file, or any file under a dir).
pub fn is_git_tracked(project_dir: &Path, rel: &str) -> bool {
    Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", rel])
        .current_dir(project_dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve `.git/info/exclude` via `git rev-parse --git-path` so worktrees and
/// non-standard git dirs work. Errors when not inside a git repository.
fn exclude_path(project_dir: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(project_dir)
        .output()
        .context("run git rev-parse (is git installed?)")?;
    if !out.status.success() {
        return Err(anyhow!(
            "not a git repository — private mode hides artifacts via .git/info/exclude, so there is nothing to hide here"
        ));
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = PathBuf::from(&raw);
    Ok(if path.is_absolute() {
        path
    } else {
        project_dir.join(path)
    })
}

fn render_block() -> String {
    let mut block = String::new();
    block.push_str(EXCLUDE_BEGIN);
    block.push('\n');
    for entry in EXCLUDE_ENTRIES {
        block.push_str(entry);
        block.push('\n');
    }
    block.push_str(EXCLUDE_END);
    block.push('\n');
    block
}

/// Remove the managed block, preserving user content around it verbatim.
fn strip_block(content: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in content.lines() {
        if line == EXCLUDE_BEGIN {
            inside = true;
            continue;
        }
        if line == EXCLUDE_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Enable private mode: write the managed exclude block + the `setup.private`
/// marker. Refuses when `whetstone/` is already tracked (the repo is already
/// publicly onboarded — exclude cannot hide tracked files). Idempotent.
pub fn enable(project_dir: &Path) -> Result<Value> {
    let exclude = exclude_path(project_dir)?;
    if is_git_tracked(project_dir, "whetstone") {
        return Err(anyhow!(
            "whetstone/ is already git-tracked — this project is already publicly onboarded. \
             Private mode is a pre-adoption state; nothing was changed."
        ));
    }

    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    let already = existing.contains(EXCLUDE_BEGIN);
    if !already {
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&render_block());
        if let Some(parent) = exclude.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&exclude, content)
            .with_context(|| format!("write {}", exclude.display()))?;
    }

    crate::onboard::set_private(project_dir, true)?;

    Ok(json!({
        "status": "ok",
        "action": if already { "noop" } else { "enabled" },
        "exclude_file": exclude.display().to_string(),
        "entries": EXCLUDE_ENTRIES,
        "next_command": "Whetstone artifacts are now invisible to git status. When the team is ready to share them, run `wh publish`.",
    }))
}

/// The flip: remove the exclude block, write real `.gitignore` entries for the
/// machine-local dirs, clear the marker, complete wiring skipped while private,
/// and PRINT what to `git add` — publish never runs git itself. Idempotent.
pub fn publish(project_dir: &Path, ci: bool, schedule: &str) -> Result<Value> {
    let exclude = exclude_path(project_dir)?;
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    let had_block = existing.contains(EXCLUDE_BEGIN);

    if !had_block && !is_private(project_dir) {
        return Ok(json!({
            "status": "ok",
            "action": "noop",
            "reason": "not in private mode — nothing to publish",
        }));
    }

    if had_block {
        std::fs::write(&exclude, strip_block(&existing))
            .with_context(|| format!("write {}", exclude.display()))?;
    }

    // The entries `wh init --personal` would have added: .state/.personal/metrics
    // stay ignored after the flip.
    let gitignore = crate::personal::ensure_gitignore_entries(project_dir)?;

    crate::onboard::set_private(project_dir, false)?;

    // Complete wiring that private mode skipped or redirected, now that sharing
    // is intended: shared MCP registration and hooks in settings.json, with our
    // entries migrated out of settings.local.json.
    let mcp = crate::onboard::register_mcp(project_dir)?;
    let hooks = crate::onboard::install_hooks_step(project_dir)?;
    let migrated = crate::triggers::remove_whetstone_hooks_from_local(project_dir)?;

    let ci_result = if ci {
        Some(crate::triggers::install_ci_workflow(project_dir, schedule)?)
    } else {
        None
    };

    // What exists and is now trackable.
    let candidates = [
        "whetstone",
        ".mcp.json",
        ".claude/settings.json",
        ".claude/whetstone-session-hook.sh",
        ".claude/whetstone-posttooluse-hook.sh",
        ".cursor/whetstone-session.md",
        ".githooks/post-merge",
        ".gitignore",
        ".github/workflows/whetstone-check.yml",
    ];
    let publish_files: Vec<String> = candidates
        .iter()
        .filter(|rel| project_dir.join(rel).exists())
        .map(|rel| rel.to_string())
        .collect();

    Ok(json!({
        "status": "ok",
        "action": "published",
        "exclude_file": exclude.display().to_string(),
        "gitignore": gitignore,
        "mcp": mcp,
        "hooks": hooks,
        "hooks_migrated_from_local": migrated,
        "ci": ci_result,
        "publish_files": publish_files,
        "next_command": format!(
            "Review the artifacts, then share them: git add {}",
            publish_files.join(" ")
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_block_preserves_user_content() {
        let user = "node_modules/\n*.log\n";
        let mut content = String::from(user);
        content.push_str(&render_block());
        content.push_str("dist/\n");
        let stripped = strip_block(&content);
        assert_eq!(stripped, "node_modules/\n*.log\ndist/\n");
        assert!(!stripped.contains("whetstone"));
    }

    #[test]
    fn strip_block_on_content_without_block_is_identity() {
        let user = "target/\n";
        assert_eq!(strip_block(user), user);
    }

    #[test]
    fn render_block_is_fenced_and_stripped_clean() {
        let block = render_block();
        assert!(block.starts_with(EXCLUDE_BEGIN));
        assert!(block.trim_end().ends_with(EXCLUDE_END));
        assert_eq!(strip_block(&block), "");
    }
}
