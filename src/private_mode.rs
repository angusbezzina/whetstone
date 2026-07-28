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

/// Repo-root-relative artifact paths. Anchored (leading `/`) when rendered so
/// they match the project directory only, never a same-named path elsewhere in
/// the tree. Static content keeps enable/publish exactly inverse operations;
/// excluding an already-tracked path is a harmless no-op.
const EXCLUDE_ENTRIES: &[&str] = &[
    "whetstone/",
    ".mcp.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".claude/whetstone-session-hook.sh",
    ".claude/whetstone-posttooluse-hook.sh",
    ".cursor/whetstone-session.md",
    ".githooks/post-merge",
];

/// True when `rel` must be left alone: we are private AND the repo tracks it.
/// `.git/info/exclude` cannot hide modifications to tracked files, so writing
/// one would put a visible diff in a teammate's `git status` — and, for a
/// committed hook script, destroy their content. Guarded call sites:
/// `.mcp.json`, `.claude/settings.json`, both `.claude/whetstone-*.sh` scripts,
/// `.cursor/whetstone-session.md`, `.githooks/post-merge`.
pub fn skip_tracked(project_dir: &Path, rel: &str) -> bool {
    is_private(project_dir) && is_git_tracked(project_dir, rel)
}

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

/// Path of `project_dir` relative to the repository root, as a `/`-terminated
/// prefix (empty at the root). The exclude file lives at the ROOT, so entries
/// for a package inside a monorepo must carry this prefix or they match nothing
/// — the artifacts would be fully exposed while `wh` reported success.
fn repo_prefix(project_dir: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_dir)
        .output()
        .context("run git rev-parse (is git installed?)")?;
    if !out.status.success() {
        return Err(anyhow!(
            "not a git repository — private mode hides artifacts via .git/info/exclude, so there is nothing to hide here"
        ));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    let root = root.canonicalize().unwrap_or(root);
    let proj = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    let rel = proj.strip_prefix(&root).unwrap_or(Path::new(""));
    let mut prefix = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    if !prefix.is_empty() {
        prefix.push('/');
    }
    Ok(prefix)
}

fn render_block(prefix: &str) -> String {
    let mut block = String::new();
    block.push_str(EXCLUDE_BEGIN);
    block.push('\n');
    for entry in EXCLUDE_ENTRIES {
        block.push('/');
        block.push_str(prefix);
        block.push_str(entry);
        block.push('\n');
    }
    block.push_str(EXCLUDE_END);
    block.push('\n');
    block
}

/// The managed block currently present, if any (used to detect a torn or
/// stale block so `enable` can repair it).
fn existing_block(content: &str) -> Option<String> {
    let start = content.find(EXCLUDE_BEGIN)?;
    let rest = &content[start..];
    let end = rest.find(EXCLUDE_END)?;
    Some(rest[..end + EXCLUDE_END.len() + 1].to_string())
}

/// Write `content` to `path` via a temp file + rename, so an interrupted write
/// never leaves a half-written exclude file.
pub(crate) fn atomic_write_str(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("wh-tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
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
    let prefix = repo_prefix(project_dir)?;
    if is_git_tracked(project_dir, "whetstone") {
        return Err(anyhow!(
            "whetstone/ is already git-tracked — this project is already publicly onboarded. \
             Private mode is a pre-adoption state; nothing was changed."
        ));
    }

    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    let wanted = render_block(&prefix);
    // Self-healing: a block whose entries don't match what we'd write now (a
    // torn write, or a block written for a different project_dir) is REPLACED,
    // not trusted. Trusting the marker alone would silently leave artifacts
    // exposed on a re-run.
    let current = existing_block(&existing);
    let had_marker = existing.contains(EXCLUDE_BEGIN);
    let action = match current.as_deref() {
        Some(b) if b == wanted => "noop",
        _ if had_marker => "repaired",
        _ => "enabled",
    };
    if action != "noop" {
        // `had_marker` without a complete block means a torn write; strip_block
        // drops from the marker to the terminator (or EOF), clearing it.
        let mut content = if had_marker {
            strip_block(&existing)
        } else {
            existing
        };
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&wanted);
        atomic_write_str(&exclude, &content)?;
    }

    crate::onboard::set_private(project_dir, true)?;

    // A pre-existing hook setup we must not silently disable (MAJOR 3): report
    // it so the user can decide, and leave core.hooksPath alone.
    let mut warnings: Vec<String> = Vec::new();
    if crate::triggers::has_local_git_hooks(project_dir) {
        warnings.push(
            "this repo has executable hooks in .git/hooks/ — Whetstone left core.hooksPath alone, so its post-merge advisory will not run (your existing hooks keep working)".to_string(),
        );
    }

    Ok(json!({
        "status": "ok",
        "action": action,
        "exclude_file": exclude.display().to_string(),
        "path_prefix": prefix,
        "entries": EXCLUDE_ENTRIES,
        "warnings": warnings,
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
        atomic_write_str(&exclude, &strip_block(&existing))?;
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
        content.push_str(&render_block(""));
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
        let block = render_block("");
        assert!(block.starts_with(EXCLUDE_BEGIN));
        assert!(block.trim_end().ends_with(EXCLUDE_END));
        assert_eq!(strip_block(&block), "");
    }

    /// The exclude file lives at the repo root, so a package inside a monorepo
    /// must carry its path prefix or the entries match nothing.
    #[test]
    fn render_block_anchors_entries_under_the_project_prefix() {
        let root = render_block("");
        assert!(root.contains("\n/whetstone/\n"));
        assert!(root.contains("\n/.mcp.json\n"));

        let nested = render_block("packages/api/");
        assert!(nested.contains("\n/packages/api/whetstone/\n"), "{nested}");
        assert!(nested.contains("\n/packages/api/.mcp.json\n"), "{nested}");
        assert!(!nested.contains("\n/whetstone/\n"), "must not anchor at root: {nested}");
        assert_eq!(strip_block(&nested), "");
    }

    /// A block that doesn't match what we'd write now (torn write, or written
    /// for a different project dir) must be detected so `enable` repairs it.
    #[test]
    fn existing_block_detects_a_torn_block() {
        let torn = format!("user\n{EXCLUDE_BEGIN}\n/whetstone/\n");
        assert!(existing_block(&torn).is_none(), "unterminated block is not a valid block");

        let complete = format!("user\n{}", render_block(""));
        let found = existing_block(&complete).expect("complete block found");
        assert_eq!(found, render_block(""));
        assert_ne!(found, render_block("pkg/"), "prefix mismatch must be visible");
    }
}
