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
/// What the `setup.private` marker says. `Indeterminate` exists because reading
/// "public" from a config we cannot understand silently disengages every
/// tracked-file guard — `wh init --hooks` then modified a teammate's committed
/// settings.json while reporting success. Fail SAFE, never open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerState {
    /// No config file: the ordinary not-yet-private case.
    Absent,
    /// A well-formed config that does not set `setup.private: true`.
    Public,
    Private,
    /// Present, but unreadable / unparseable / not the expected shape. Treated
    /// as private everywhere, because the guards that keeps engaged only ever
    /// SKIP writes to tracked files.
    Indeterminate,
}

pub fn marker_state(project_dir: &Path) -> MarkerState {
    let path = project_dir.join("whetstone").join("whetstone.yaml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return if path.exists() {
            MarkerState::Indeterminate
        } else {
            MarkerState::Absent
        };
    };
    let Ok(doc) = serde_yaml::from_str::<Value>(&raw) else {
        return MarkerState::Indeterminate;
    };
    // A config that is not a mapping (a list, a bare scalar, an empty file) tells
    // us nothing about the mode — it is not "public".
    let Some(obj) = doc.as_object() else {
        return MarkerState::Indeterminate;
    };
    match obj.get("setup") {
        // No `setup` block at all is the normal public config.
        None => MarkerState::Public,
        Some(setup) => {
            let Some(setup_obj) = setup.as_object() else {
                // `setup: null`, `setup: "x"` — malformed, not public.
                return MarkerState::Indeterminate;
            };
            match setup_obj.get("private") {
                None => MarkerState::Public,
                Some(v) => match v.as_bool() {
                    Some(true) => MarkerState::Private,
                    Some(false) => MarkerState::Public,
                    // `private: "yes"` — a value we will not guess at.
                    None => MarkerState::Indeterminate,
                },
            }
        }
    }
}

pub fn is_private(project_dir: &Path) -> bool {
    matches!(
        marker_state(project_dir),
        MarkerState::Private | MarkerState::Indeterminate
    )
}

/// True if git tracks anything matching `rel` (a file, or any file under a dir).
pub fn is_git_tracked(project_dir: &Path, rel: &str) -> bool {
    let exact = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", rel])
        .current_dir(project_dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if exact {
        return true;
    }
    // On a case-folding filesystem (macOS APFS by default) a tracked
    // `.MCP.json` IS the file we would write as `.mcp.json`, but git's
    // pathspec matching is case-sensitive — so the exact check says
    // "untracked" and we would clobber a committed team file.
    let want = rel.trim_end_matches('/').to_lowercase();
    Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split('\0')
                .any(|p| {
                    let p = p.to_lowercase();
                    p == want || p.starts_with(&format!("{want}/"))
                })
        })
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

/// True when no process with this pid is alive, i.e. the lock is an orphan.
/// The mtime-only staleness check this replaces could never fire: the threshold
/// (30s) exceeded the whole retry budget (5s), so a lock left by a `kill -9`
/// wedged every later run for 30s and blamed "another `wh` process".
fn holder_is_gone(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // `kill -0` probes liveness without signalling.
        !Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(true)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn lock_exclude(exclude: &Path) -> Result<ExcludeLock> {
    let lock = exclude.with_extension("wh-lock");
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The pid is written into a temp file and hard-linked into place, so the
    // lock is NEVER observable without its owner's pid — a plain create-then-
    // write leaves a window where a peer reads it empty and cannot tell an
    // orphan from a live holder.
    let stamp = std::process::id();
    for attempt in 0..400 {
        let tmp = lock.with_extension(format!("wh-lock-{stamp}-{attempt}"));
        let _ = std::fs::remove_file(&tmp);
        // Keep the OS cause in the message itself: the JSON error surface renders
        // only the top-level string, so a bare "write <path>" hid "Permission
        // denied" on a read-only .git/info. Same shape as `read_exclude` — a
        // match, not `map_err`, which Whetstone's own anyhow rule forbids.
        if let Err(e) = std::fs::write(&tmp, stamp.to_string()) {
            return Err(anyhow!("write {}: {e}", tmp.display()));
        }
        match std::fs::hard_link(&tmp, &lock) {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                return Ok(ExcludeLock(lock));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp);
                let holder = std::fs::read_to_string(&lock)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                let reclaim = match holder {
                    // A dead holder's lock is reclaimed — but `holder_is_gone`
                    // forks, which takes long enough that the lock may already
                    // have been replaced by a LIVE peer's. Re-read and require
                    // the same pid before unlinking, so we never steal a valid
                    // lock. (The verify-and-retry around every write is the real
                    // guarantee; this just keeps the window small.)
                    Some(pid) => {
                        holder_is_gone(pid)
                            && std::fs::read_to_string(&lock)
                                .ok()
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                == Some(pid)
                    }
                    // No parseable pid: either a foreign file or a crash from
                    // before this scheme. Reclaim only after a grace period, so
                    // it can never race a live holder mid-handshake.
                    // `symlink_metadata`: a symlinked lock must not have its
                    // TARGET's mtime consulted (a dangling link never ages out,
                    // bricking private mode forever).
                    None => std::fs::symlink_metadata(&lock)
                        .and_then(|m| m.modified())
                        .map(|t| t.elapsed().map(|d| d.as_secs() >= 5).unwrap_or(false))
                        .unwrap_or(false),
                };
                if reclaim {
                    // A lock that is a directory needs remove_dir; without this
                    // a stray `mkdir exclude.wh-lock` bricked every later run.
                    if std::fs::remove_file(&lock).is_err() {
                        let _ = std::fs::remove_dir_all(&lock);
                    }
                    continue;
                }
                if attempt == 399 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            // Not every filesystem supports hard links (FAT, some network
            // mounts). Fall back to the plain atomic create — it leaves a brief
            // window where a peer sees the lock without a pid, which the grace
            // period above already handles — rather than refusing to lock at all.
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        let _ = f.write_all(stamp.to_string().as_bytes());
                        return Ok(ExcludeLock(lock));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        if attempt == 399 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    Err(e) => {
                        return Err(e).with_context(|| format!("create {}", lock.display()))
                    }
                }
            }
        }
    }
    Err(anyhow!(
        "timed out waiting for {} — another `wh` process is updating it",
        lock.display()
    ))
}

/// Read-modify-write the exclude file, then VERIFY the intent actually landed —
/// retrying if a concurrent writer clobbered it.
///
/// The lock alone is not sufficient, and round 11 proved it: any scheme that can
/// reclaim an orphaned lock can, in a narrow window, reclaim a LIVE holder's, and
/// then two writers interleave a read-modify-write of one shared file. The loser's
/// update vanishes — silently leaving a package exposed (enable) or its block
/// behind while `publish` reports success (publish). Verification makes
/// correctness independent of the lock's judgment: each writer re-reads after
/// releasing and retries until its own intent is present, so concurrent writers
/// with different labels converge instead of losing updates.
fn update_exclude_verified(
    exclude: &Path,
    compose: impl Fn(&str) -> String,
    landed: impl Fn(&str) -> bool,
    what: &str,
) -> Result<()> {
    for attempt in 0..8 {
        {
            let _lock = lock_exclude(exclude)?;
            let existing = read_exclude(exclude)?;
            let next = compose(&existing);
            if next != existing {
                atomic_write_str(exclude, &next)?;
            }
            // Lock released here on purpose: the re-read below must be able to
            // observe a concurrent writer's clobber, which is exactly what we
            // are retrying against.
        }
        if landed(&read_exclude(exclude)?) {
            return Ok(());
        }
        // Jittered by attempt so two contending processes don't resynchronize.
        std::thread::sleep(std::time::Duration::from_millis(10 + 15 * attempt as u64));
    }
    Err(anyhow!(
        "could not {what} {} — a concurrent `wh` process kept overwriting it. Re-run.",
        exclude.display()
    ))
}

/// True when `line` is EXACTLY one of the entries we render for `label`'s
/// project. Scoped and exact on purpose: a suffix match (`ends_with("/.mcp.json")`)
/// claimed the user's own `/tools/legacy/.mcp.json` as ours and silently deleted
/// it on publish — unrecoverable data loss in the one file we promise to preserve
/// verbatim. Used to bound a TORN block, which would otherwise eat every user
/// line to EOF.
fn is_managed_entry(line: &str, label: &str) -> bool {
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
    // The prefix this label's entries carry ("" at the repo root).
    let prefix = if label == ROOT_LABEL {
        String::new()
    } else {
        format!("{label}/")
    };
    EXCLUDE_ENTRIES
        .iter()
        .any(|entry| unescaped == format!("{prefix}{entry}"))
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
                            || is_managed_entry(l, label))
                    })
                    .collect();
                lines.splice(begin..=end, kept);
            }
            // Torn (no terminator): drop the fence and the entries that follow
            // it, stopping at the first foreign line so user ignores survive.
            None => {
                let mut i = begin + 1;
                while i < lines.len() && is_managed_entry(lines[i], label) {
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
    let label = label_for(&prefix);
    let wanted = render_block(&prefix);
    // Did the project config exist before this call? Decides whether a refusal
    // may delete it during revert.
    let had_config = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists();

    // State BEFORE any write, read under the lock, so `action` and the revert
    // decision below describe what this call actually found.
    let (had_marker, action) = {
        let _lock = lock_exclude(&exclude)?;
        let existing = read_exclude(&exclude)?;
        let had_marker = has_marker(&existing, &label);
        let action = match existing_block(&existing, &label).as_deref() {
            Some(b) if b == wanted => "noop",
            _ if had_marker => "repaired",
            _ => "enabled",
        };
        (had_marker, action)
    };

    // Self-healing, scoped to OUR label: a block whose entries don't match what
    // we'd write now (a torn write, or hand-edited entries) is REPLACED, not
    // trusted. Trusting the marker alone would silently leave artifacts
    // exposed on a re-run. Sibling packages' blocks are never touched.
    update_exclude_verified(
        &exclude,
        |existing| {
            // `has_marker` without a complete block means a torn write;
            // strip_block clears it before we append the fresh one.
            let mut content = if has_marker(existing, &label) {
                strip_block(existing, &label)
            } else {
                existing.to_string()
            };
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&wanted);
            content
        },
        |after| existing_block(after, &label).as_deref() == Some(wanted.as_str()),
        "add this project's block to",
    )?;

    crate::onboard::set_private(project_dir, true)?;

    // Ask GIT whether the promise actually holds, rather than trusting that we
    // wrote the right patterns. An in-tree .gitignore negation, a stale block,
    // or a lost concurrent write would otherwise leak silently — the one
    // failure mode private mode must never have.
    let exposed = exposed_artifacts(project_dir, &prefix)?;
    let (blocking, advisory) = partition_exposures(&exposed);
    if !blocking.is_empty() {
        // Leave no half-private repo behind. A refusal that still wrote the
        // block and the marker left the project flagged private with nothing
        // onboarded, and the caller aborts before the remaining steps run — so
        // undo exactly what THIS call created. A repo that was already private
        // keeps its state; only its error is reported.
        if !had_marker {
            let _ = update_exclude_verified(
                &exclude,
                |existing| strip_block(existing, &label),
                |after| !has_marker(after, &label),
                "revert this project's block in",
            );
            let _ = crate::onboard::set_private(project_dir, false);
            // `set_private` had to CREATE whetstone/whetstone.yaml to hold the
            // marker. Clearing the key left `?? whetstone/` visible on the
            // shared repo — a footprint from a call that refused to do anything.
            // Remove what we created, never what was already there.
            if !had_config {
                let ws_dir = project_dir.join("whetstone");
                let _ = std::fs::remove_file(ws_dir.join("whetstone.yaml"));
                // Only if empty: a concurrent step may have written packs.
                let _ = std::fs::remove_dir(&ws_dir);
            }
        }
        return Err(anyhow!(
            "private mode is NOT fully in effect — git can still see:\n  {}\n\
             Resolve the above, then re-run. To leave private mode entirely, run `wh publish`.",
            blocking.join("\n  ")
        ));
    }

    let mut warnings: Vec<String> = shared_exclude_warning(project_dir).into_iter().collect();
    warnings.extend(advisory);

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

/// Does this artifact carry Whetstone's fingerprint?
///
/// Attribution is answered by CONTENT, not by git state. Four consecutive
/// releases shipped a hole trying to infer "did we write this?" from the index,
/// from HEAD, or from porcelain status letters — each fix missed a state nobody
/// enumerated (`A `, then intent-to-add ` A`, then unmerged `AA`, then a
/// root-relative path handed to a cwd-relative pathspec). Worse, the premise
/// itself was false: "present in HEAD ⇒ not ours" assumed every artifact has a
/// tracked-file guard, and `.claude/settings.local.json` had none.
///
/// A file either bears our mark or it does not. That question has the same
/// answer in every git state, at any directory depth, on any filesystem.
fn artifact_is_ours(repo_root: &Path, rel: &str) -> bool {
    // `.gitignore` is the user's file that we merely APPEND a fenced block to,
    // so it is ours only when it carries that exact marker. A substring match
    // here made any hand-written `whetstone/` ignore line — the most natural
    // first move a cautious solo adopter makes — read as our leak, and refused
    // onboarding with a false diagnosis.
    if is_gitignore(rel) {
        return file_has_marker(repo_root, rel);
    }
    // Paths we exclusively own: nothing else in a repo is called these.
    if rel.to_lowercase().contains("whetstone") {
        return true;
    }
    // Shared-name files (.mcp.json, .claude/settings*.json, .githooks/post-merge)
    // are ours only if our content is actually in them.
    std::fs::read_to_string(repo_root.join(rel))
        .map(|s| s.to_lowercase().contains("whetstone"))
        .unwrap_or(false)
}

/// A repo-root-relative path naming a `.gitignore` (at any depth).
fn is_gitignore(rel: &str) -> bool {
    Path::new(rel).file_name().and_then(|n| n.to_str()) == Some(".gitignore")
}

fn file_has_marker(repo_root: &Path, rel: &str) -> bool {
    std::fs::read_to_string(repo_root.join(rel))
        .map(|s| s.contains(crate::personal::GITIGNORE_MARKER))
        .unwrap_or(false)
}

/// True when the COMMITTED copy already bears our mark — our content is public
/// history, so a working-tree change to that file is the user's edit, not our
/// leak. `git show HEAD:<rel>` takes a root-relative path, which is exactly
/// what `git status --porcelain` reports.
fn head_copy_is_ours(repo_root: &Path, rel: &str) -> bool {
    Command::new("git")
        .args(["show", &format!("HEAD:{rel}")])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout);
            // Same asymmetry as `artifact_is_ours`: for `.gitignore` only our
            // fenced marker counts, never a bare mention of the name.
            if is_gitignore(rel) {
                text.contains(crate::personal::GITIGNORE_MARKER)
            } else {
                text.to_lowercase().contains("whetstone")
            }
        })
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
/// One artifact git can still see. `blocking` separates a genuine leak (rules,
/// config, agent wiring — private mode must refuse) from an advisory one: a
/// `.gitignore` carrying our personal-layer block holds only ignore lines, never
/// rules or taste, and is a legitimately shared file. Blocking on it made
/// `enable → publish → enable` impossible, contradicting the design contract.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Exposure {
    pub path: String,
    pub hint: String,
    pub blocking: bool,
}

impl Exposure {
    pub fn display(&self) -> String {
        format!("{} ({})", self.path, self.hint)
    }
}

/// Split exposures into blocking and advisory display lines.
pub fn partition_exposures(exposed: &[Exposure]) -> (Vec<String>, Vec<String>) {
    let mut blocking = Vec::new();
    let mut advisory = Vec::new();
    for e in exposed {
        if e.blocking {
            blocking.push(e.display());
        } else {
            advisory.push(e.display());
        }
    }
    (blocking, advisory)
}

pub fn exposed_artifacts(project_dir: &Path, prefix: &str) -> Result<Vec<Exposure>> {
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
    // `git status --porcelain` paths are REPO-ROOT-relative, so every path
    // check below must be too — regardless of how deep `project_dir` sits.
    let repo_root = repo_toplevel(project_dir)?;
    let candidates: Vec<String> = EXCLUDE_ENTRIES
        .iter()
        .chain(SHARED_ARTIFACTS.iter())
        .map(|e| format!("{prefix}{e}"))
        .chain(std::iter::once(format!("{prefix}.gitignore")))
        .collect();

    // Case-insensitively: on a case-folding filesystem `.MCP.json` and
    // `.mcp.json` are the same file, so a case variant must still match.
    let matches = |path: &str| {
        let p = path.to_lowercase();
        candidates.iter().any(|o| {
            let o = o.to_lowercase();
            let o_dir = o.trim_end_matches('/');
            p == o || p == o_dir || p.starts_with(&format!("{o_dir}/"))
        })
    };

    let mut exposed = Vec::new();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut records = stdout.split('\0');
    while let Some(rec) = records.next() {
        if rec.len() < 4 {
            continue;
        }
        let (code, path) = rec.split_at(3);
        // A rename/copy record is followed by a bare field holding the ORIGINAL
        // path. Consume it, or it gets rescanned as a status record and its
        // first three characters are read as a status code.
        if code.starts_with('R') || code.starts_with('C') {
            let _ = records.next();
        }
        if !matches(path) {
            continue;
        }
        // Content decides authorship — no status-code or index/HEAD inference.
        if !artifact_is_ours(&repo_root, path) {
            continue;
        }
        // ...unless our content is already committed. Then it is public
        // history, and a working-tree change to it is the user's edit.
        if head_copy_is_ours(&repo_root, path) {
            continue;
        }
        let (hint, blocking) = if is_gitignore(path) {
            (
                "carries Whetstone's personal-layer block (ignore lines only — no rules or config), \
                 which cannot be hidden: commit it, or remove those lines",
                false,
            )
        } else if path.to_lowercase().contains("workflows/whetstone-check.yml") {
            (
                "a Whetstone CI workflow is inherently shared — delete it, or commit it and accept that it is public",
                true,
            )
        } else {
            (
                "a Whetstone artifact is visible to git — if you staged it, `git restore --staged <path>`; otherwise an in-tree .gitignore may re-include it, and `git check-ignore -v <path>` shows which rule wins",
                true,
            )
        };
        exposed.push(Exposure {
            path: path.to_string(),
            hint: hint.to_string(),
            blocking,
        });
    }
    Ok(exposed)
}

/// The repository root. Every verifier path is relative to this.
fn repo_toplevel(project_dir: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_dir)
        .output()
        .context("run git rev-parse")?;
    if !out.status.success() {
        return Err(anyhow!("not a git repository"));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    Ok(root.canonicalize().unwrap_or(root))
}

/// The flip: remove the exclude block, write real `.gitignore` entries for the
/// machine-local dirs, clear the marker, complete wiring skipped while private,
/// and PRINT what to `git add` — publish never runs git itself. Idempotent.
pub fn publish(project_dir: &Path, ci: bool, schedule: &str) -> Result<Value> {
    let exclude = exclude_path(project_dir)?;
    let prefix = repo_prefix(project_dir)?;
    let label = label_for(&prefix);
    guard_symlinked_into_worktree(project_dir, &exclude)?;
    let had_block = {
        let _lock = lock_exclude(&exclude)?;
        has_marker(&read_exclude(&exclude)?, &label)
    };

    if !had_block && !is_private(project_dir) {
        return Ok(json!({
            "status": "ok",
            "action": "noop",
            "reason": "not in private mode — nothing to publish",
        }));
    }

    // Only our own block — a sibling package may still be private. Verified and
    // retried: a lost update here left the block in place while publish reported
    // success, printing a `git add` list that git then refused (round 11).
    if had_block {
        update_exclude_verified(
            &exclude,
            |existing| strip_block(existing, &label),
            |after| !has_marker(after, &label),
            "remove this project's block from",
        )?;
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

        assert!(is_managed_entry("/whetstone/", "."));
        assert!(is_managed_entry(
            "/packages/api/.claude/settings.json",
            "packages/api"
        ));
        assert!(is_managed_entry("/pkg\\[1\\]/whetstone/", "pkg[1]"));
        assert!(!is_managed_entry("keep-b", "."));
        assert!(!is_managed_entry("/my-whetstone-notes/", "."));
    }

    /// Entries are matched EXACTLY for this label's project. A suffix match
    /// claimed the user's own same-named paths and publish deleted them.
    #[test]
    fn managed_entry_match_is_scoped_and_exact() {
        // The user's own file that merely ENDS with one of our names.
        assert!(!is_managed_entry("/tools/legacy/.mcp.json", "."));
        assert!(!is_managed_entry("/vendor/x/.githooks/post-merge", "."));
        assert!(!is_managed_entry("/my/own/whetstone/", "."));
        // Another project's entry is not ours.
        assert!(!is_managed_entry("/packages/api/.mcp.json", "."));
        assert!(!is_managed_entry("/.mcp.json", "packages/api"));
    }

    /// Publish must not delete a user line parked inside our fenced block just
    /// because it ends with one of our filenames.
    #[test]
    fn strip_block_keeps_user_lines_that_merely_end_with_our_names() {
        let content = format!(
            "{}\n/whetstone/\n/tools/legacy/.mcp.json\nKEEP-ME/\n{}\n",
            begin_line("."),
            end_line(".")
        );
        let stripped = strip_block(&content, ".");
        assert!(
            stripped.contains("/tools/legacy/.mcp.json"),
            "the user's own path must survive: {stripped}"
        );
        assert!(stripped.contains("KEEP-ME/"), "foreign lines survive: {stripped}");
        assert!(!stripped.contains("/whetstone/"), "our entry goes: {stripped}");
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
