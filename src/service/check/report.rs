//! The check report: one result per requirement (the compiled-in scanner and
//! each rule's gate) and the state they add up to. Only a pass of every
//! required result is success; a check with nothing required is unknown.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::{ContentDigest, RecordRef};

pub const REPORT_SCHEMA: &str = "whetstone.check-report.v1";
const MAX_FINDINGS: usize = 256;
const MAX_TEXT_BYTES: usize = 2_048;
const MAX_CODE_FILES: usize = 4_096;
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
    /// Content digest of relevant paths and bytes, not merely Git HEAD.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckResult {
    pub(crate) requirement_id: String,
    pub(crate) required: bool,
    pub(crate) state: CheckState,
    pub(crate) summary: String,
    pub(crate) evidence: EvidencePointer,
    pub(crate) findings: Vec<Finding>,
    pub(crate) governing_records: Vec<RecordRef>,
    pub(crate) repair_direction: String,
    pub(crate) permitted_next_action: String,
    pub(crate) recheck_command: String,
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
            result.governing_records.sort();
            result.governing_records.dedup();
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
                    result.state.label(),
                    result.requirement_id,
                    result.summary
                ));
            } else {
                lines.push(format!(
                    "{} {}: {} — next: {}",
                    result.state.label(),
                    result.requirement_id,
                    result.summary,
                    result.permitted_next_action
                ));
            }
        }
        lines.join("\n")
    }
}

/// Hash relevant repository paths including uncommitted file bytes. Symlinks
/// and paths outside the canonical project root are refused.
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
    let unsafe_path = |path: &Path| format!("unsafe code path: {}", path.display());
    let mut files = Vec::new();
    for relative in relevant_paths {
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(unsafe_path(relative));
        }
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(unsafe_path(relative));
        }
        let canonical = path.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(&root) {
            return Err(unsafe_path(relative));
        }
        if metadata.is_file() {
            files.push(canonical);
        } else if metadata.is_dir() {
            for entry in walkdir::WalkDir::new(&canonical)
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
                let metadata =
                    std::fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
                if metadata.file_type().is_symlink() {
                    return Err(unsafe_path(entry.path()));
                }
                if metadata.is_file() {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            return Err(unsafe_path(relative));
        }
    }
    files.sort();
    files.dedup();
    let mut bytes = 0_u64;
    for file in &files {
        let size = file.metadata().map_err(|error| error.to_string())?.len();
        bytes = bytes
            .checked_add(size)
            .ok_or("the code snapshot is too large")?;
    }
    if files.len() > MAX_CODE_FILES || bytes > MAX_CODE_BYTES {
        return Err("the code snapshot exceeds its file or byte bounds".into());
    }
    let mut hash = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(&root).map_err(|_| unsafe_path(&file))?;
        let name = relative.to_string_lossy();
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update(
            file.metadata()
                .map_err(|error| error.to_string())?
                .len()
                .to_be_bytes(),
        );
        let mut reader = File::open(&file).map_err(|error| error.to_string())?;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let read = reader.read(&mut chunk).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            hash.update(&chunk[..read]);
        }
    }
    Ok(digest_of(hash))
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
            governing_records: Vec::new(),
            repair_direction: "repair".into(),
            permitted_next_action: format!("repair {id}"),
            recheck_command: "wh check".into(),
        }
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
    fn the_code_digest_follows_content_and_refuses_escapes() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(temp.path().join("lib.rs"), "pub fn one() {}\n").expect("write");
        let paths = vec![PathBuf::from(".")];
        let first = code_tree_digest(temp.path(), &paths).expect("digest");
        assert_eq!(first, code_tree_digest(temp.path(), &paths).expect("again"));
        std::fs::write(temp.path().join("lib.rs"), "pub fn two() {}\n").expect("write");
        assert_ne!(
            first,
            code_tree_digest(temp.path(), &paths).expect("changed")
        );
        assert!(code_tree_digest(temp.path(), &[PathBuf::from("../outside")]).is_err());
    }
}
