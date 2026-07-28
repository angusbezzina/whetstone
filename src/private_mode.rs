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

/// Blocks are LABELLED with the project's path relative to the repo root, so a
/// monorepo can have several private packages at once: each owns its own fenced
/// block and enable/publish only ever touch their own label. An unlabelled
/// legacy fence is treated as the repo root.
const BEGIN_PREFIX: &str = "# >>> whetstone private mode";
const END_PREFIX: &str = "# <<< whetstone private mode";
const ROOT_LABEL: &str = ".";

fn begin_line(label: &str) -> String {
    format!("{BEGIN_PREFIX} [{label}] (managed by `wh`; `wh publish` removes this block) >>>")
}

fn end_line(label: &str) -> String {
    format!("{END_PREFIX} [{label}] <<<")
}

/// The label a fence line carries, if it is one of ours. `kind` is the fence
/// prefix to match. Unlabelled (pre-label) fences resolve to the root label.
fn fence_label(line: &str, kind: &str) -> Option<String> {
    let rest = line.strip_prefix(kind)?;
    Some(match (rest.find('['), rest.find(']')) {
        (Some(a), Some(b)) if b > a => rest[a + 1..b].to_string(),
        _ => ROOT_LABEL.to_string(),
    })
}

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

/// The block's label: the prefix without its trailing slash, or `.` at the root.
fn label_for(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        ROOT_LABEL.to_string()
    } else {
        trimmed.to_string()
    }
}

/// `.git/info/exclude` is gitignore syntax, so glob metacharacters in a real
/// directory name (`pkg[1]`, `a*b`) must be escaped or the pattern silently
/// matches nothing — exposing everything while `wh` reports success.
fn escape_glob(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn render_block(prefix: &str) -> String {
    let label = label_for(prefix);
    let escaped = escape_glob(prefix);
    let mut block = String::new();
    block.push_str(&begin_line(&label));
    block.push('\n');
    for entry in EXCLUDE_ENTRIES {
        block.push('/');
        block.push_str(&escaped);
        block.push_str(entry);
        block.push('\n');
    }
    block.push_str(&end_line(&label));
    block.push('\n');
    block
}

/// This project's managed block, if present (used to detect a torn or stale
/// block so `enable` can repair it). Other projects' blocks are invisible here.
fn existing_block(content: &str, label: &str) -> Option<String> {
    let mut collecting = false;
    let mut out = String::new();
    for line in content.lines() {
        if !collecting {
            if fence_label(line, BEGIN_PREFIX).as_deref() == Some(label) {
                collecting = true;
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if fence_label(line, END_PREFIX).as_deref() == Some(label) {
            return Some(out);
        }
        // A new BEGIN before our END means the block was torn.
        if fence_label(line, BEGIN_PREFIX).is_some() {
            return None;
        }
    }
    None
}

/// True if any fence for `label` appears — including a torn one with no END.
fn has_marker(content: &str, label: &str) -> bool {
    content
        .lines()
        .any(|l| fence_label(l, BEGIN_PREFIX).as_deref() == Some(label))
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

/// Remove only THIS project's managed block. User content and any other
/// project's block are preserved verbatim — a monorepo may have several
/// packages private at once, and clobbering a sibling's block would silently
/// expose its artifacts.
fn strip_block(content: &str, label: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in content.lines() {
        if !inside {
            if fence_label(line, BEGIN_PREFIX).as_deref() == Some(label) {
                inside = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Inside our block: drop lines until our END. A different project's
        // BEGIN means ours was torn — stop dropping and keep this line.
        if fence_label(line, END_PREFIX).as_deref() == Some(label) {
            inside = false;
            continue;
        }
        if fence_label(line, BEGIN_PREFIX).is_some() {
            inside = false;
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
    let label = label_for(&prefix);
    let wanted = render_block(&prefix);
    // Self-healing, scoped to OUR label: a block whose entries don't match what
    // we'd write now (a torn write, or hand-edited entries) is REPLACED, not
    // trusted. Trusting the marker alone would silently leave artifacts
    // exposed on a re-run. Sibling packages' blocks are never touched.
    let current = existing_block(&existing, &label);
    let had_marker = has_marker(&existing, &label);
    let action = match current.as_deref() {
        Some(b) if b == wanted => "noop",
        _ if had_marker => "repaired",
        _ => "enabled",
    };
    if action != "noop" {
        // `had_marker` without a complete block means a torn write; strip_block
        // drops from our marker to our terminator (or the next fence), clearing it.
        let mut content = if had_marker {
            strip_block(&existing, &label)
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

    Ok(json!({
        "status": "ok",
        "action": action,
        "exclude_file": exclude.display().to_string(),
        "path_prefix": prefix,
        "label": label,
        "entries": EXCLUDE_ENTRIES,
        "next_command": "Whetstone artifacts are now invisible to git status. When the team is ready to share them, run `wh publish`.",
    }))
}

/// The flip: remove the exclude block, write real `.gitignore` entries for the
/// machine-local dirs, clear the marker, complete wiring skipped while private,
/// and PRINT what to `git add` — publish never runs git itself. Idempotent.
pub fn publish(project_dir: &Path, ci: bool, schedule: &str) -> Result<Value> {
    let exclude = exclude_path(project_dir)?;
    let prefix = repo_prefix(project_dir)?;
    let label = label_for(&prefix);
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    let had_block = has_marker(&existing, &label);

    if !had_block && !is_private(project_dir) {
        return Ok(json!({
            "status": "ok",
            "action": "noop",
            "reason": "not in private mode — nothing to publish",
        }));
    }

    // Only our own block — a sibling package may still be private.
    if had_block {
        atomic_write_str(&exclude, &strip_block(&existing, &label))?;
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
        let stripped = strip_block(&content, ".");
        assert_eq!(stripped, "node_modules/\n*.log\ndist/\n");
        assert!(!stripped.contains("whetstone"));
    }

    #[test]
    fn strip_block_on_content_without_block_is_identity() {
        let user = "target/\n";
        assert_eq!(strip_block(user, "."), user);
    }

    #[test]
    fn render_block_is_fenced_and_stripped_clean() {
        let block = render_block("");
        assert!(block.starts_with(&begin_line(".")));
        assert!(block.trim_end().ends_with(&end_line(".")));
        assert_eq!(strip_block(&block, "."), "");
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
        assert_eq!(strip_block(&nested, "packages/api"), "");
    }

    /// A block that doesn't match what we'd write now (torn write, or
    /// hand-edited entries) must be detected so `enable` repairs it.
    #[test]
    fn existing_block_detects_a_torn_block() {
        let torn = format!("user\n{}\n/whetstone/\n", begin_line("."));
        assert!(
            existing_block(&torn, ".").is_none(),
            "unterminated block is not a valid block"
        );
        assert!(has_marker(&torn, "."), "but the marker is still detectable");

        let complete = format!("user\n{}", render_block(""));
        let found = existing_block(&complete, ".").expect("complete block found");
        assert_eq!(found, render_block(""));
    }

    /// Several packages in one monorepo may be private at once: each owns a
    /// labelled block, and enable/publish must never touch a sibling's.
    #[test]
    fn blocks_for_different_projects_coexist() {
        let api = render_block("packages/api/");
        let web = render_block("packages/web/");
        let content = format!("user\n{api}{web}");

        assert_eq!(
            existing_block(&content, "packages/api").as_deref(),
            Some(api.as_str())
        );
        assert_eq!(
            existing_block(&content, "packages/web").as_deref(),
            Some(web.as_str())
        );

        // Publishing api must leave web's block completely intact.
        let after = strip_block(&content, "packages/api");
        assert!(!after.contains("/packages/api/whetstone/"), "api entries gone: {after}");
        assert!(after.contains("/packages/web/whetstone/"), "web entries kept: {after}");
        assert!(after.contains("user"), "user content kept: {after}");
        assert_eq!(existing_block(&after, "packages/web").as_deref(), Some(web.as_str()));
    }

    /// `.git/info/exclude` is glob syntax — an unescaped `[` in a real
    /// directory name makes the pattern match nothing, exposing everything.
    #[test]
    fn glob_metacharacters_in_the_prefix_are_escaped() {
        let block = render_block("pkg[1]/");
        assert!(block.contains("/pkg\\[1\\]/whetstone/"), "{block}");
        assert_eq!(escape_glob("a*b?c[d]e\\f"), "a\\*b\\?c\\[d\\]e\\\\f");
        // The label stays human-readable (it is a comment, not a pattern).
        assert!(block.contains("[pkg[1]]"), "{block}");
    }

    /// An unlabelled fence from before labels existed is treated as the root.
    #[test]
    fn legacy_unlabelled_fence_resolves_to_root() {
        assert_eq!(
            fence_label("# >>> whetstone private mode (managed by `wh`) >>>", BEGIN_PREFIX)
                .as_deref(),
            Some(ROOT_LABEL)
        );
        assert_eq!(
            fence_label(&begin_line("packages/api"), BEGIN_PREFIX).as_deref(),
            Some("packages/api")
        );
        assert_eq!(fence_label("# unrelated comment", BEGIN_PREFIX), None);
    }
}
