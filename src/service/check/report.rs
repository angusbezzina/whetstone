//! The check report: one result per requirement (each rule's gate) and the
//! state they add up to. Only a pass of every
//! required result is success; a check with nothing required is unknown.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::ContentDigest;
use crate::rulebook::Revision;

pub const REPORT_SCHEMA: &str = "whetstone.check-report.v1";
const MAX_FINDINGS: usize = 256;
const MAX_TEXT_BYTES: usize = 2_048;
const MAX_CODE_BYTES: u64 = 64 * 1024 * 1024;
const SNAPSHOT_SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
];

/// What a check ran against: code, policy, checker, scope, environment and
/// trust, each as a content digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SnapshotBinding {
    /// Git's tree plus the changed paths' bytes, or the walked paths' bytes
    /// outside Git.
    pub(crate) code_tree: ContentDigest,
    pub(crate) policy: ContentDigest,
    pub(crate) checker_bundle: ContentDigest,
    pub(crate) scope: ContentDigest,
    pub(crate) environment: ContentDigest,
    pub(crate) trust: ContentDigest,
}

/// Where a result's evidence lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EvidencePointer {
    pub(crate) source: String,
    pub(crate) locator: String,
    pub(crate) digest: ContentDigest,
}

/// One flag with the repair route back to green.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Finding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column: Option<u32>,
    pub(crate) rule_id: String,
    pub(crate) observed: String,
    pub(crate) expected: String,
    pub(crate) rationale: String,
    pub(crate) repair_direction: String,
    pub(crate) permitted_next_action: String,
    pub(crate) recheck_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckState {
    Success,
    Violated,
    Unknown,
    Unavailable,
}

impl CheckState {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "PASS",
            Self::Violated => "FAIL",
            Self::Unknown => "UNKNOWN",
            Self::Unavailable => "UNAVAILABLE",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Violated => "violated",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The rule a result enforces: its id, the digest of its canonical JSON and
/// the revision of the rulebook it was read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleReference {
    pub(crate) id: String,
    pub(crate) digest: ContentDigest,
    pub(crate) revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckResult {
    pub(crate) requirement_id: String,
    pub(crate) required: bool,
    pub(crate) state: CheckState,
    pub(crate) summary: String,
    pub(crate) evidence: EvidencePointer,
    pub(crate) findings: Vec<Finding>,
    pub(crate) governing_rule: RuleReference,
    pub(crate) repair_direction: String,
    pub(crate) permitted_next_action: String,
    pub(crate) recheck_command: String,
}

impl CheckResult {
    /// The word the summary shows: a should rule that flags reports and
    /// never blocks, so it reads WARN rather than a blocking FAIL.
    fn label(&self) -> &'static str {
        if self.state == CheckState::Violated && !self.required {
            "WARN"
        } else {
            self.state.label()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckReport {
    pub(crate) schema: &'static str,
    pub(crate) report_id: ContentDigest,
    pub(crate) subject: String,
    pub(crate) snapshot: SnapshotBinding,
    pub(crate) evaluated_at_unix: u64,
    pub(crate) state: CheckState,
    pub(crate) summary: String,
    pub(crate) results: Vec<CheckResult>,
    pub(crate) next_actions: Vec<String>,
}

impl CheckReport {
    /// Add the results up: a broken must rule is violated, an unavailable one
    /// unavailable, and anything else short of every required pass unknown.
    pub(crate) fn new(
        subject: String,
        snapshot: SnapshotBinding,
        mut results: Vec<CheckResult>,
        evaluated_at_unix: u64,
    ) -> Self {
        results.sort_by(|left, right| left.requirement_id.cmp(&right.requirement_id));
        for result in &mut results {
            result.summary = bounded(&result.summary);
            result.findings.truncate(MAX_FINDINGS);
            for finding in &mut result.findings {
                for text in [
                    &mut finding.observed,
                    &mut finding.expected,
                    &mut finding.rationale,
                    &mut finding.repair_direction,
                ] {
                    *text = bounded(text);
                }
            }
            result.findings.sort_by(|left, right| {
                (&left.file, left.line, left.column, &left.rule_id).cmp(&(
                    &right.file,
                    right.line,
                    right.column,
                    &right.rule_id,
                ))
            });
        }
        let required = results
            .iter()
            .filter(|result| result.required)
            .collect::<Vec<_>>();
        let state = if required
            .iter()
            .any(|result| result.state == CheckState::Violated)
        {
            CheckState::Violated
        } else if required
            .iter()
            .any(|result| result.state == CheckState::Unavailable)
        {
            CheckState::Unavailable
        } else if required.is_empty()
            || required
                .iter()
                .any(|result| result.state != CheckState::Success)
        {
            CheckState::Unknown
        } else {
            CheckState::Success
        };
        let summary = match state {
            CheckState::Success => "Every applicable must rule holds.",
            CheckState::Violated => "A must rule is broken.",
            CheckState::Unavailable => "A must rule's enforcer is unavailable.",
            CheckState::Unknown => {
                "A must rule's evidence is missing, skipped or incomplete, which is not a pass."
            }
        };
        let mut next_actions = required
            .iter()
            .filter(|result| result.state != CheckState::Success)
            .map(|result| result.permitted_next_action.clone())
            .collect::<Vec<_>>();
        next_actions.sort();
        next_actions.dedup();
        let mut report = Self {
            schema: REPORT_SCHEMA,
            report_id: zero_digest(),
            subject,
            snapshot,
            evaluated_at_unix,
            state,
            summary: summary.into(),
            results,
            next_actions,
        };
        let bytes = serde_json::to_vec(&report).unwrap_or_default();
        report.report_id = sha256_digest(&bytes);
        report
    }

    pub(crate) fn human_summary(&self) -> String {
        let mut lines = vec![format!("{} ({})", self.summary, self.state.name())];
        for result in &self.results {
            if result.state == CheckState::Success {
                lines.push(format!(
                    "{} {}: {}",
                    result.label(),
                    result.requirement_id,
                    result.summary
                ));
            } else {
                lines.push(format!(
                    "{} {}: {} — next: {}",
                    result.label(),
                    result.requirement_id,
                    result.summary,
                    result.permitted_next_action
                ));
            }
        }
        lines.join("\n")
    }
}

/// Identify the code under the relevant paths. In a Git work tree this is
/// Git's own view, the HEAD tree plus the bytes of every changed or untracked
/// path under them, so its cost follows the change rather than the
/// repository; elsewhere the paths are walked. A symlink is its link text, as
/// Git stores it, and is never followed. Paths outside the canonical project
/// root are refused.
pub(crate) fn code_tree_digest(
    project_root: &Path,
    relevant_paths: &[PathBuf],
) -> Result<ContentDigest, String> {
    let root = project_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !root.is_dir() || relevant_paths.is_empty() {
        return Err("the code snapshot needs a project directory and at least one path".into());
    }
    let mut paths = Vec::new();
    for relative in relevant_paths {
        if !safe_relative(relative) {
            return Err(unsafe_path(relative));
        }
        let canonical = root
            .join(relative)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !canonical.starts_with(&root) {
            return Err(unsafe_path(relative));
        }
        paths.push(canonical);
    }
    paths.sort();
    paths.dedup();
    let mut hash = Sha256::new();
    match git_toplevel(&root) {
        Some(top) => digest_git_view(&mut hash, &top, &paths)?,
        None => digest_walk(&mut hash, &root, &paths)?,
    }
    Ok(digest_of(hash))
}

fn unsafe_path(path: &Path) -> String {
    format!("unsafe code path: {}", path.display())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn git(cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
}

fn git_toplevel(root: &Path) -> Option<PathBuf> {
    let output = git(root, &["rev-parse", "--show-toplevel"])?;
    PathBuf::from(String::from_utf8_lossy(&output).trim_end_matches('\n'))
        .canonicalize()
        .ok()
        .filter(|top| root.starts_with(top))
}

/// The HEAD tree id, then each path Git reports as changed against HEAD or
/// untracked, with its bytes.
fn digest_git_view(hash: &mut Sha256, top: &Path, paths: &[PathBuf]) -> Result<(), String> {
    // An unborn branch has no tree; every file is then untracked or staged.
    let tree = git(top, &["rev-parse", "--verify", "-q", "HEAD^{tree}"]).unwrap_or_default();
    hash.update(b"git-tree\0");
    hash.update(&tree);
    let mut args = vec![
        "status".to_string(),
        "--porcelain=v1".into(),
        "-z".into(),
        "--untracked-files=all".into(),
        "--no-renames".into(),
        "--".into(),
    ];
    for path in paths {
        let relative = path.strip_prefix(top).map_err(|_| unsafe_path(path))?;
        args.push(if relative.as_os_str().is_empty() {
            ":(top)".into()
        } else {
            format!(":(top,literal){}", relative.to_string_lossy())
        });
    }
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let status = git(top, &args).ok_or("git could not report the working tree")?;
    let mut changed = status
        .split(|byte| *byte == 0)
        .filter(|entry| entry.len() > 3)
        .map(|entry| String::from_utf8_lossy(&entry[3..]).to_string())
        .collect::<Vec<_>>();
    changed.sort();
    changed.dedup();
    for name in changed {
        let relative = Path::new(name.trim_end_matches('/'));
        if !safe_relative(relative) {
            return Err(unsafe_path(relative));
        }
        digest_entry(hash, &name, &top.join(relative))?;
    }
    Ok(())
}

/// Every file and symlink under the paths, skipping tool and build
/// directories, within the byte bound.
fn digest_walk(hash: &mut Sha256, root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let mut entries = Vec::new();
    for path in paths {
        for entry in walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !SNAPSHOT_SKIP_DIRS
                        .iter()
                        .any(|name| entry.file_name() == *name)
            })
        {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry.file_type().is_dir() {
                entries.push(entry.into_path());
            }
        }
    }
    entries.sort();
    entries.dedup();
    let mut bytes = 0_u64;
    for path in entries {
        let relative = path.strip_prefix(root).map_err(|_| unsafe_path(&path))?;
        bytes = bytes.saturating_add(digest_entry(hash, &relative.to_string_lossy(), &path)?);
        if bytes > MAX_CODE_BYTES {
            return Err("the code snapshot exceeds its byte bound".into());
        }
    }
    Ok(())
}

/// One named entry: a file's bytes, a symlink's link text, or a marker for
/// a directory (a nested repository) or a deleted path. Returns the file
/// bytes read.
fn digest_entry(hash: &mut Sha256, name: &str, path: &Path) -> Result<u64, String> {
    hash.update((name.len() as u64).to_be_bytes());
    hash.update(name.as_bytes());
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hash.update(b"\0deleted");
            return Ok(0);
        }
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path).map_err(|error| error.to_string())?;
        let target = target.to_string_lossy();
        hash.update(b"\0link");
        hash.update((target.len() as u64).to_be_bytes());
        hash.update(target.as_bytes());
        return Ok(0);
    }
    if !metadata.is_file() {
        hash.update(b"\0other");
        return Ok(0);
    }
    hash.update(b"\0file");
    hash.update(metadata.len().to_be_bytes());
    let mut reader = File::open(path).map_err(|error| error.to_string())?;
    let mut chunk = [0_u8; 64 * 1024];
    let mut read_bytes = 0_u64;
    loop {
        let read = reader.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        read_bytes += read as u64;
        hash.update(&chunk[..read]);
    }
    Ok(read_bytes)
}

fn bounded(value: &str) -> String {
    if value.len() <= MAX_TEXT_BYTES {
        return value.to_string();
    }
    let mut boundary = MAX_TEXT_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...[TRUNCATED]", &value[..boundary])
}

fn sha256_digest(bytes: &[u8]) -> ContentDigest {
    let mut hash = Sha256::new();
    hash.update(bytes);
    digest_of(hash)
}

fn digest_of(hash: Sha256) -> ContentDigest {
    match ContentDigest::new(format!("sha256:{:x}", hash.finalize())) {
        Ok(digest) => digest,
        Err(_) => unreachable!("SHA-256 formatting always satisfies the digest contract"),
    }
}

fn zero_digest() -> ContentDigest {
    match ContentDigest::new(format!("sha256:{}", "0".repeat(64))) {
        Ok(digest) => digest,
        Err(_) => unreachable!("a zero digest satisfies the digest contract"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SnapshotBinding {
        SnapshotBinding {
            code_tree: zero_digest(),
            policy: zero_digest(),
            checker_bundle: zero_digest(),
            scope: zero_digest(),
            environment: zero_digest(),
            trust: zero_digest(),
        }
    }

    fn result(id: &str, required: bool, state: CheckState) -> CheckResult {
        CheckResult {
            requirement_id: id.into(),
            required,
            state,
            summary: "ran".into(),
            evidence: EvidencePointer {
                source: "test".into(),
                locator: id.into(),
                digest: zero_digest(),
            },
            findings: Vec::new(),
            governing_rule: RuleReference {
                id: id.into(),
                digest: zero_digest(),
                revision: Revision::Uncommitted,
            },
            repair_direction: "repair".into(),
            permitted_next_action: format!("repair {id}"),
            recheck_command: "wh check".into(),
        }
    }

    #[test]
    fn a_should_flag_reads_warn_and_a_must_flag_reads_fail() {
        let report = CheckReport::new(
            "subject".into(),
            snapshot(),
            vec![
                result("rule.must", true, CheckState::Violated),
                result("rule.should", false, CheckState::Violated),
                result("rule.ok", true, CheckState::Success),
            ],
            1,
        );
        let summary = report.human_summary();
        assert!(summary.contains("\nFAIL rule.must: ran"), "{summary}");
        assert!(summary.contains("\nWARN rule.should: ran"), "{summary}");
        assert!(summary.contains("\nPASS rule.ok: ran"), "{summary}");
    }

    #[test]
    fn only_every_required_pass_is_success() {
        let report = |results| CheckReport::new("subject".into(), snapshot(), results, 1).state;
        assert_eq!(
            report(vec![result("a", true, CheckState::Success)]),
            CheckState::Success
        );
        assert_eq!(
            report(vec![
                result("a", true, CheckState::Success),
                result("b", false, CheckState::Violated)
            ]),
            CheckState::Success,
            "a should flag never fails the check"
        );
        assert_eq!(
            report(vec![
                result("a", true, CheckState::Violated),
                result("b", true, CheckState::Unavailable)
            ]),
            CheckState::Violated
        );
        assert_eq!(
            report(vec![
                result("a", true, CheckState::Success),
                result("b", true, CheckState::Unavailable)
            ]),
            CheckState::Unavailable
        );
        assert_eq!(
            report(vec![
                result("a", true, CheckState::Success),
                result("b", true, CheckState::Unknown)
            ]),
            CheckState::Unknown
        );
        assert_eq!(
            report(vec![result("a", false, CheckState::Success)]),
            CheckState::Unknown
        );
        assert_eq!(report(Vec::new()), CheckState::Unknown);
    }

    #[test]
    fn the_report_is_deterministic_and_bounded() {
        let mut long = result("b", true, CheckState::Violated);
        long.summary = "x".repeat(5_000);
        let first = CheckReport::new(
            "subject".into(),
            snapshot(),
            vec![long.clone(), result("a", true, CheckState::Success)],
            7,
        );
        let second = CheckReport::new(
            "subject".into(),
            snapshot(),
            vec![result("a", true, CheckState::Success), long],
            7,
        );
        assert_eq!(first, second);
        assert_eq!(first.results[0].requirement_id, "a");
        assert!(first.results[1].summary.ends_with("[TRUNCATED]"));
        assert_eq!(first.next_actions, vec!["repair b".to_string()]);
    }

    #[test]
    fn the_walked_digest_follows_content_and_link_text_and_refuses_escapes() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(temp.path().join("lib.rs"), "pub fn one() {}\n").expect("write");
        std::os::unix::fs::symlink("lib.rs", temp.path().join("alias.rs")).expect("link");
        let paths = vec![PathBuf::from(".")];
        let first = code_tree_digest(temp.path(), &paths).expect("digest");
        assert_eq!(first, code_tree_digest(temp.path(), &paths).expect("again"));
        std::fs::write(temp.path().join("lib.rs"), "pub fn two() {}\n").expect("write");
        let second = code_tree_digest(temp.path(), &paths).expect("changed");
        assert_ne!(first, second);
        std::fs::remove_file(temp.path().join("alias.rs")).expect("unlink");
        std::os::unix::fs::symlink("other.rs", temp.path().join("alias.rs")).expect("relink");
        assert_ne!(
            second,
            code_tree_digest(temp.path(), &paths).expect("relinked"),
            "a symlink is its link text"
        );
        assert!(code_tree_digest(temp.path(), &[PathBuf::from("../outside")]).is_err());
    }

    fn git_ok(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}");
    }

    #[test]
    fn the_git_digest_follows_the_tree_and_the_changed_bytes() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        let outside = tempfile::tempdir().expect("outside");
        git_ok(root, &["init", "-q"]);
        std::fs::create_dir(root.join("src")).expect("src");
        std::fs::write(root.join("src/lib.rs"), "pub fn one() {}\n").expect("write");
        std::os::unix::fs::symlink("src", root.join("alias")).expect("link");
        std::os::unix::fs::symlink(outside.path(), root.join("away")).expect("link out");
        let paths = vec![PathBuf::from(".")];
        let untracked = code_tree_digest(root, &paths).expect("unborn");
        git_ok(root, &["add", "."]);
        git_ok(
            root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-qm",
                "start",
            ],
        );
        let clean = code_tree_digest(root, &paths).expect("clean");
        assert_ne!(untracked, clean);
        assert_eq!(clean, code_tree_digest(root, &paths).expect("again"));
        std::fs::write(root.join("src/lib.rs"), "pub fn two() {}\n").expect("write");
        let dirty = code_tree_digest(root, &paths).expect("dirty");
        assert_ne!(clean, dirty);
        std::fs::write(root.join("src/new.rs"), "pub fn three() {}\n").expect("write");
        assert_ne!(dirty, code_tree_digest(root, &paths).expect("untracked"));
        assert!(code_tree_digest(root, &[PathBuf::from("alias")]).is_ok());
        assert!(code_tree_digest(root, &[PathBuf::from("away")]).is_err());
        assert!(code_tree_digest(root, &[PathBuf::from("../outside")]).is_err());
    }
}
