//! Sweep findings: `wh check --sweep --format findings` runs only the rules
//! below must (should, advisory, or in shadow) and turns each flag into one
//! finding for a night sweep's controller to work (Messo's, inside Virgil).
//!
//! A finding is `{fingerprint, subjectDigest, class, title, responsibility}`.
//! The fingerprint (rule id, repository and subject) stays the same across
//! runs for the same subject, so a dismissed finding stays quiet until its
//! subject changes; the subject digest changes exactly when the flagged
//! content does. No timestamps, no run ids: the same repository state gives
//! byte-identical findings.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::{Enforcer, Rule, Strength, VerificationAxis};
use crate::gates::GateOutcome;

use super::SelectedRule;

/// One flag, as the sweep's controller reads it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct SweepFinding {
    pub(crate) fingerprint: String,
    #[serde(rename = "subjectDigest")]
    pub(crate) subject_digest: String,
    pub(crate) class: &'static str,
    pub(crate) title: String,
    pub(crate) responsibility: String,
}

/// Whether a rule's flags are sweep work: below must (or a must rule still in
/// shadow, which blocks nobody), and enforced from the repository's content.
/// Drive proofs and reviews are not swept: they are the feature sweep and a
/// person's verdict.
pub(crate) fn sweepable(rule: &Rule) -> bool {
    (rule.strength != Strength::Must || rule.in_shadow())
        && !matches!(
            rule.enforcer,
            Enforcer::Drive { .. } | Enforcer::Review { .. } | Enforcer::Brief { .. }
        )
}

/// The repository's identity in a fingerprint: its origin remote when it has
/// one (the same on every clone), its directory name otherwise.
pub(crate) fn repository_identity(project: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|remote| !remote.is_empty())
        .map(|remote| {
            remote
                .trim_end_matches(".git")
                .replace("git@", "")
                .replace("https://", "")
                .replace(':', "/")
        })
        .unwrap_or_else(|| super::super::project_label(project))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// The flagged path and line in a failure location (`src/a.rs:12:4`), when
/// the location names a file in the repository.
fn located(project: &Path, location: &str) -> Option<(String, Option<usize>)> {
    let mut parts = location.split(':');
    let path = parts.next()?.trim().trim_start_matches("./");
    if path.is_empty() || !crate::domain::safe_relative_path(path) {
        return None;
    }
    if !project.join(path).is_file() {
        return None;
    }
    let line = parts
        .next()
        .and_then(|line| line.trim().parse::<usize>().ok());
    Some((path.to_string(), line))
}

/// What was flagged: the flagged line's text when the location names one,
/// the whole file when it names only a file.
fn flagged_content(project: &Path, path: &str, line: Option<usize>) -> Vec<u8> {
    let bytes = std::fs::read(project.join(path)).unwrap_or_default();
    match line {
        Some(line) if line > 0 => String::from_utf8_lossy(&bytes)
            .lines()
            .nth(line - 1)
            .map(|text| text.trim().as_bytes().to_vec())
            .unwrap_or(bytes),
        _ => bytes,
    }
}

/// One finding per flag of every swept rule, sorted.
pub(crate) fn findings(
    project: &Path,
    selected: &[SelectedRule],
    outcomes: &[GateOutcome],
) -> Vec<SweepFinding> {
    let repository = repository_identity(project);
    let rules = selected
        .iter()
        .map(|selected| (selected.id.as_str(), &selected.rule))
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    for outcome in outcomes {
        if outcome.state != VerificationAxis::Fail {
            continue;
        }
        let Some(rule) = rules.get(outcome.id.as_str()) else {
            continue;
        };
        if !sweepable(rule) {
            continue;
        }
        // (subject, digest of what was flagged, where to say it is)
        let mut flagged = Vec::<(String, String, String)>::new();
        for failure in &outcome.failures {
            match located(project, &failure.location) {
                Some((path, line)) => flagged.push((
                    path.clone(),
                    digest(&flagged_content(project, &path, line)),
                    path,
                )),
                None => flagged.push((
                    "repository".into(),
                    digest(format!("{}\0{}", failure.location, failure.message).as_bytes()),
                    failure.location.clone(),
                )),
            }
        }
        if flagged.is_empty() {
            flagged.push((
                "repository".into(),
                digest(outcome.summary.as_bytes()),
                "the repository".into(),
            ));
        }
        // The same rule flagging one subject twice: the second is `#2`, so
        // each flag is its own finding and the first keeps its fingerprint.
        flagged.sort();
        let mut seen = BTreeMap::<String, usize>::new();
        for (subject, subject_digest, place) in flagged {
            let count = seen.entry(subject.clone()).or_insert(0);
            *count += 1;
            let subject = if *count == 1 {
                subject
            } else {
                format!("{subject}#{count}")
            };
            result.push(SweepFinding {
                fingerprint: digest(format!("{}\0{repository}\0{subject}", outcome.id).as_bytes()),
                subject_digest,
                class: rule.sweep_class().label(),
                title: format!("{} ({place})", rule.statement)
                    .chars()
                    .take(300)
                    .collect(),
                responsibility: outcome.id.clone(),
            });
        }
    }
    result.sort();
    result.dedup();
    result
}

/// JSON lines, one finding per line, for the controller to read.
pub(crate) fn json_lines(findings: &[SweepFinding]) -> String {
    findings
        .iter()
        .filter_map(|finding| serde_json::to_string(finding).ok())
        .map(|line| line + "\n")
        .collect()
}
