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

/// Labels are delimited by `[...]`, so a path component containing a bracket
/// would truncate on read — and a truncated label matches a SIBLING's block,
/// whose entries then get deleted. Percent-encode the three characters that
/// could break the round-trip; ordinary paths are unchanged.
fn encode_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for ch in label.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            _ => out.push(ch),
        }
    }
    out
}

fn begin_line(label: &str) -> String {
    format!(
        "{BEGIN_PREFIX} [{}] (managed by `wh`; `wh publish` removes this block) >>>",
        encode_label(label)
    )
}

fn end_line(label: &str) -> String {
    format!("{END_PREFIX} [{}] <<<", encode_label(label))
}

/// The encoded label a fence line carries, if it is one of ours. `kind` is the
/// fence prefix to match. Unlabelled (pre-label) fences resolve to the root.
/// Safe against brackets in paths because the written label is encoded.
fn fence_label(line: &str, kind: &str) -> Option<String> {
    let rest = line.trim_end_matches(['\n', '\r']).strip_prefix(kind)?;
    Some(match (rest.find('['), rest.find(']')) {
        (Some(a), Some(b)) if b > a => rest[a + 1..b].to_string(),
        _ => ROOT_LABEL.to_string(),
    })
}

/// True when this fence line belongs to `label`.
fn fence_is(line: &str, kind: &str, label: &str) -> bool {
    fence_label(line, kind).as_deref() == Some(encode_label(label).as_str())
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

/// Artifacts Whetstone writes that are INHERENTLY SHARED and therefore must
/// never be hidden — but must still be accounted for, or `wh` would report
/// "invisible to git status" with one of its own files plainly visible.
/// `wh init --ci` before going private is the reachable path.
const SHARED_ARTIFACTS: &[&str] = &[".github/workflows/whetstone-check.yml"];

/// Read the exclude file as text. Git treats it as bytes, so a single non-UTF-8
/// byte (a latin-1 pattern) used to make `read_to_string` fail and the file be
/// treated as EMPTY — destroying the user's personal ignores on enable, and
/// turning publish into a silent no-op that still reported success. Missing is
/// fine (empty); unreadable or non-UTF-8 is an error.
fn read_exclude(path: &Path) -> Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).with_context(|| {
            format!(
                "{} contains non-UTF-8 bytes — Whetstone will not rewrite it, because doing so \
                 would discard content it cannot represent. Convert the file to UTF-8 and re-run.",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        // Keep the OS cause in the message itself — the JSON error surface only
        // renders the top-level string, so a bare "read <path>" hides why.
        Err(e) => Err(anyhow!("read {}: {e}", path.display())),
    }
}

/// Refuse when `.git/info/exclude` is a symlink pointing at a file git TRACKS:
/// writing through it would modify a tracked file, which private mode promises
/// never to do (and which the artifact-scoped verifier cannot see).
fn guard_symlinked_into_worktree(project_dir: &Path, exclude: &Path) -> Result<()> {
    // Only a real symlink can redirect our write somewhere unexpected. (The
    // exclude file itself lives under `<root>/.git/`, which is inside the repo
    // path but not part of the working tree — so a plain path must not trip
    // the worktree check below.)
    match std::fs::symlink_metadata(exclude) {
        Ok(meta) if meta.file_type().is_symlink() => {}
        _ => return Ok(()),
    }
    let Ok(target) = std::fs::canonicalize(exclude) else {
        return Ok(());
    };
    let git_dir = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        .map(|p| p.canonicalize().unwrap_or(p));
    if let Some(git_dir) = git_dir {
        if target.starts_with(&git_dir) {
            return Ok(()); // Still inside the git dir — invisible to git status.
        }
    }
    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_dir)
        .output()
    else {
        return Ok(());
    };
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    let root = root.canonicalize().unwrap_or(root);
    let Ok(rel) = target.strip_prefix(&root) else {
        return Ok(()); // Outside the worktree (the normal dotfiles case) — fine.
    };
    // Tracked or not: a file inside the working tree is visible to `git status`
    // (or is a tracked file we would be modifying), so writing our block into
    // it breaks the promise either way.
    Err(anyhow!(
        ".git/info/exclude is a symlink to {}, which is inside the working tree — writing the \
         managed block there would put it in `git status` (or modify a tracked file). \
         Point the symlink outside the repository.",
        rel.to_string_lossy()
    ))
}

/// True when this clone has more than one worktree. They SHARE
/// `.git/info/exclude` (it lives in the common dir), so one block covers them
/// all and `wh publish` in any worktree un-hides the others.
fn worktree_count(project_dir: &Path) -> usize {
    Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.starts_with("worktree "))
                .count()
        })
        .unwrap_or(1)
}

fn shared_exclude_warning(project_dir: &Path) -> Option<String> {
    let n = worktree_count(project_dir);
    (n > 1).then(|| {
        format!(
            "this clone has {n} worktrees, which SHARE .git/info/exclude — one block covers all of them, \
             so `wh publish` in any worktree makes these artifacts visible in every worktree"
        )
    })
}

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
            if fence_is(line, BEGIN_PREFIX, label) {
                collecting = true;
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if fence_is(line, END_PREFIX, label) {
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
        .any(|l| fence_is(l, BEGIN_PREFIX, label))
}

/// Write `content` to `path` via a temp file + rename, so an interrupted write
/// never leaves a half-written exclude file. The temp name is unique per
/// process+call — a shared one races with a concurrent `wh` and both lose.
/// Writes THROUGH a symlink (a dotfiles setup often links `exclude`) and
/// restores the original file mode after the rename.
pub(crate) fn atomic_write_str(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    // Resolve a symlink so we replace its TARGET, not the link itself.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mode = file_mode(&target);
    let tmp = target.with_extension(format!(
        "wh-tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("write {}", target.display()));
    }
    restore_mode(&target, mode);
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn restore_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
}

#[cfg(not(unix))]
fn restore_mode(_path: &Path, _mode: Option<u32>) {}

/// Held for the whole read-modify-write of the exclude file. Two `wh` processes
/// onboarding different packages of one monorepo otherwise interleave and one
/// package's block is silently lost — with its `setup.private` still true.
struct ExcludeLock(PathBuf);

impl Drop for ExcludeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lock_exclude(exclude: &Path) -> Result<ExcludeLock> {
    let lock = exclude.with_extension("wh-lock");
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for attempt in 0..200 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => return Ok(ExcludeLock(lock)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Reclaim a lock orphaned by a killed process.
                let stale = std::fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .map(|t| t.elapsed().map(|d| d.as_secs() > 30).unwrap_or(false))
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                if attempt == 199 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(e).with_context(|| format!("create {}", lock.display())),
        }
    }
    Err(anyhow!(
        "timed out waiting for {} — another `wh` process is updating it",
        lock.display()
    ))
}

/// True when `line` is one of the entries we render (for any prefix). Used to
/// bound a TORN block: without it, a block missing its terminator would eat
/// every user line that follows to EOF.
fn is_managed_entry(line: &str) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let Some(rest) = line.strip_prefix('/') else {
        return false;
    };
    let unescaped: String = {
        let mut out = String::with_capacity(rest.len());
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    EXCLUDE_ENTRIES
        .iter()
        .any(|entry| unescaped == *entry || unescaped.ends_with(&format!("/{entry}")))
}

/// Remove only THIS project's managed block. User content and any other
/// project's block are preserved verbatim — a monorepo may have several
/// packages private at once, and clobbering a sibling's block would silently
/// expose its artifacts.
fn strip_block(content: &str, label: &str) -> String {
    // `split_inclusive` keeps each line's own terminator, so CRLF endings and a
    // missing final newline survive the round-trip — `lines()` would silently
    // rewrite the user's file.
    let mut lines: Vec<&str> = content.split_inclusive('\n').collect();
    // Repeat so duplicated blocks for this label are all removed.
    while let Some(begin) = lines.iter().position(|l| fence_is(l, BEGIN_PREFIX, label)) {
        let end = lines
            .iter()
            .skip(begin + 1)
            .position(|l| fence_is(l, END_PREFIX, label))
            .map(|i| i + begin + 1);
        match end {
            // Well-formed: drop our fences and our entries across the whole
            // region, but KEEP anything foreign the user parked inside it.
            // (Dropping only up to the first foreign line would orphan the
            // rest of the block — publish must be the exact inverse of enable.)
            Some(end) => {
                let kept: Vec<&str> = lines[begin..=end]
                    .iter()
                    .copied()
                    .filter(|l| {
                        !(fence_is(l, BEGIN_PREFIX, label)
                            || fence_is(l, END_PREFIX, label)
                            || is_managed_entry(l))
                    })
                    .collect();
                lines.splice(begin..=end, kept);
            }
            // Torn (no terminator): drop the fence and the entries that follow
            // it, stopping at the first foreign line so user ignores survive.
            None => {
                let mut i = begin + 1;
                while i < lines.len() && is_managed_entry(lines[i]) {
                    i += 1;
                }
                lines.drain(begin..i);
            }
        }
    }
    // Lines already carry their own terminators — do not add any.
    lines.concat()
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

    // A newline (or other control char) in a path component would split the
    // fence line and every entry across two lines, corrupting the block — and
    // it leaks with no user misconfiguration at all. Refuse rather than write
    // something that cannot work.
    if prefix.chars().any(char::is_control) {
        return Err(anyhow!(
            "this project's path contains a control character, which cannot be expressed \
             in .git/info/exclude — private mode is not possible here. Rename the directory."
        ));
    }

    guard_symlinked_into_worktree(project_dir, &exclude)?;
    let _lock = lock_exclude(&exclude)?;
    let existing = read_exclude(&exclude)?;
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

    // Ask GIT whether the promise actually holds, rather than trusting that we
    // wrote the right patterns. An in-tree .gitignore negation, a stale block,
    // or a lost concurrent write would otherwise leak silently — the one
    // failure mode private mode must never have.
    let exposed = exposed_artifacts(project_dir, &prefix)?;
    if !exposed.is_empty() {
        return Err(anyhow!(
            "private mode is NOT fully in effect — git can still see:\n  {}\n\
             Resolve the above, then re-run. To leave private mode entirely, run `wh publish`.",
            exposed.join("\n  ")
        ));
    }

    let warnings: Vec<String> = shared_exclude_warning(project_dir).into_iter().collect();

    Ok(json!({
        "status": "ok",
        "action": action,
        "exclude_file": exclude.display().to_string(),
        "path_prefix": prefix,
        "label": label,
        "entries": EXCLUDE_ENTRIES,
        "warnings": warnings,
        "verified": true,
        "next_command": "Whetstone artifacts are now invisible to git status. When the team is ready to share them, run `wh publish`.",
    }))
}

/// True when the committed copy of `rel` already carries Whetstone's
/// personal-layer marker — i.e. the block is public history, so a working-tree
/// change to that file is the user's, not ours.
fn head_has_marker(project_dir: &Path, rel: &str) -> bool {
    Command::new("git")
        .args(["show", &format!("HEAD:{rel}")])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(crate::personal::GITIGNORE_MARKER))
        .unwrap_or(false)
}

/// The project's path prefix relative to the git root (public wrapper for
/// callers that need it to interpret `exposed_artifacts`).
pub fn project_prefix(project_dir: &Path) -> Result<String> {
    repo_prefix(project_dir)
}

/// Artifact paths git can still SEE under this project — the empirical check
/// that private mode is real. Untracked-but-ignored files do not appear in
/// `git status --porcelain`, so anything listed here is genuinely exposed.
pub fn exposed_artifacts(project_dir: &Path, prefix: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        // `--untracked-files=all`: the default collapses an untracked directory
        // to `?? .claude/`, which would hide a single re-included file inside it.
        // `-z`: NUL-separated and NEVER C-quoted. Without it git quotes any path
        // with non-ASCII bytes (core.quotePath defaults to true), a quote, a
        // backslash or a control char — and a quoted path matches no entry, so
        // the leak would be filtered away and reported as verified.
        .args(["status", "--porcelain", "--untracked-files=all", "-z"])
        .current_dir(project_dir)
        .output()
        .context("run git status to verify private mode")?;
    if !out.status.success() {
        // Fail CLOSED: a check that cannot run must never read as "nothing
        // exposed" — that is the silent-success outcome private mode forbids.
        return Err(anyhow!(
            "could not verify private mode: git status failed ({})",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let hidden: Vec<String> = EXCLUDE_ENTRIES
        .iter()
        .map(|e| format!("{prefix}{e}"))
        .collect();
    let shared: Vec<String> = SHARED_ARTIFACTS
        .iter()
        .map(|e| format!("{prefix}{e}"))
        .collect();
    let gitignore = format!("{prefix}.gitignore");
    // `.gitignore` is ours only when it carries the personal-layer block a
    // previous `wh init --personal` (or a publish) wrote. An unrelated edit by
    // the user is not our leak.
    let gitignore_is_ours = std::fs::read_to_string(project_dir.join(".gitignore"))
        .map(|s| s.contains(crate::personal::GITIGNORE_MARKER))
        .unwrap_or(false);

    let matches = |path: &str, owned: &[String]| {
        owned.iter().any(|o| {
            let o_dir = o.trim_end_matches('/');
            path == o.as_str() || path == o_dir || path.starts_with(&format!("{o_dir}/"))
        })
    };

    let mut exposed = Vec::new();
    for rec in String::from_utf8_lossy(&out.stdout).split('\0') {
        if rec.len() < 4 {
            continue;
        }
        let (code, path) = rec.split_at(3);
        // Attribution test: is this path absent from HEAD? `??` (untracked) and
        // `A*` (staged addition) are both files that did not exist in the last
        // commit, so private mode is what put them there. Only a path already
        // in HEAD (` M`, `M `, `MM`) cannot be ours, because `skip_tracked`
        // stops us writing a tracked file.
        //
        // "In the INDEX" is the wrong test and was a real leak: an artifact we
        // wrote while it was untracked, then `git add`ed (exactly what
        // `wh publish` tells the user to do), is `A ` — neither untracked nor
        // ours-by-index — and dropped out of the leak set entirely.
        let absent_from_head = code.starts_with("??") || code.starts_with('A');
        if matches(path, &hidden) {
            if absent_from_head {
                exposed.push(format!("{path} (visible to git — if it is staged, `git restore --staged {path}`; otherwise an in-tree .gitignore may re-include it, and `git check-ignore -v {path}` shows which rule wins)"));
            }
        } else if matches(path, &shared) {
            exposed.push(format!(
                "{path} (a Whetstone CI workflow is inherently shared — delete it, or commit it and accept that it is public)"
            ));
        } else if path == gitignore && gitignore_is_ours {
            // Same attribution rule. If HEAD's copy ALREADY carries the marker,
            // the block is committed and public: a later modification is the
            // user's own edit, not ours, and hard-failing on it would wedge
            // every `wh init` (the round-6 fix applied to only half of this).
            let committed_block = !absent_from_head && head_has_marker(project_dir, &gitignore);
            if !committed_block {
                exposed.push(format!(
                    "{path} (carries Whetstone's personal-layer block, which cannot be hidden — remove those lines, or commit them and accept that they are public)"
                ));
            }
        }
    }
    Ok(exposed)
}

/// The flip: remove the exclude block, write real `.gitignore` entries for the
/// machine-local dirs, clear the marker, complete wiring skipped while private,
/// and PRINT what to `git add` — publish never runs git itself. Idempotent.
pub fn publish(project_dir: &Path, ci: bool, schedule: &str) -> Result<Value> {
    let exclude = exclude_path(project_dir)?;
    let prefix = repo_prefix(project_dir)?;
    let label = label_for(&prefix);
    guard_symlinked_into_worktree(project_dir, &exclude)?;
    let _lock = lock_exclude(&exclude)?;
    let existing = read_exclude(&exclude)?;
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
        "warnings": shared_exclude_warning(project_dir)
            .into_iter()
            .collect::<Vec<_>>(),
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
        // The label percent-encodes brackets so it round-trips (see
        // labels_round_trip_through_brackets); ordinary paths are untouched.
        assert!(block.contains("[pkg%5B1%5D]"), "{block}");
        assert!(render_block("packages/api/").contains("[packages/api]"));
    }

    /// The label is bracket-delimited, so a path containing a bracket must
    /// encode or it truncates on read — and a truncated label collides with a
    /// SIBLING's block, deleting it.
    #[test]
    fn labels_round_trip_through_brackets() {
        for label in ["pkg[1]", "a]b", "a[b", "100%", "packages/api", "."] {
            let begin = begin_line(label);
            assert!(
                fence_is(&begin, BEGIN_PREFIX, label),
                "BEGIN must match its own label {label}: {begin}"
            );
            assert!(fence_is(&end_line(label), END_PREFIX, label), "END for {label}");
        }
        // The collision that deleted a sibling: `[a]b]` used to read back as `a`.
        assert!(!fence_is(&begin_line("a]b"), BEGIN_PREFIX, "a"));
        assert!(!fence_is(&begin_line("pkg[1]"), BEGIN_PREFIX, "pkg[1"));

        // A full block for a bracketed path is found and stripped by label only.
        let block = render_block("a]b/");
        let content = format!("user\n{block}");
        assert_eq!(existing_block(&content, "a]b").as_deref(), Some(block.as_str()));
        assert!(existing_block(&content, "a").is_none(), "must not match a sibling");
        assert_eq!(strip_block(&content, "a"), content, "sibling strip is a no-op");
    }

    /// A torn block must stop at the first line that isn't one of ours, or it
    /// eats every user ignore below it.
    #[test]
    fn torn_block_stops_at_foreign_lines() {
        let torn = format!("keep-a\n{}\n/whetstone/\nkeep-b\nkeep-c\n", begin_line("."));
        let stripped = strip_block(&torn, ".");
        assert_eq!(stripped, "keep-a\nkeep-b\nkeep-c\n", "user lines must survive");

        assert!(is_managed_entry("/whetstone/"));
        assert!(is_managed_entry("/packages/api/.claude/settings.json"));
        assert!(is_managed_entry("/pkg\\[1\\]/whetstone/"));
        assert!(!is_managed_entry("keep-b"));
        assert!(!is_managed_entry("/my-whetstone-notes/"));
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
