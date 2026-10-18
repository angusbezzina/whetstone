//! Sweep findings: `wh check --sweep --format findings` runs only the rules
//! below must (should or advisory) and turns each flag into one
//! finding for a night sweep's controller to work (Messo's, inside Virgil).
//!
//! A finding is `{fingerprint, subjectDigest, class, priority, title,
//! responsibility, unsure?}`: `priority` (0 most urgent to 4) is the rule's,
//! and `unsure` says why, only where the rule doubts its own flag (the
//! consumer parks such a finding until a person raises it).
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
    pub(crate) priority: u8,
    pub(crate) title: String,
    pub(crate) responsibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unsure: Option<String>,
}

/// Whether a rule's flags are sweep work: below must, and enforced from the
/// repository's content. Drive proofs are not swept: they are the feature
/// sweep.
pub(crate) fn sweepable(rule: &Rule) -> bool {
    rule.strength != Strength::Must && !matches!(rule.enforcer, Enforcer::Drive { .. })
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

/// The flagged path, line and column in a failure location
/// (`src/a.rs:12:4`), when the location names a file in the repository.
fn located(project: &Path, location: &str) -> Option<(String, Option<usize>, Option<usize>)> {
    let mut parts = location.split(':');
    let path = parts.next()?.trim().trim_start_matches("./");
    if path.is_empty() || !crate::domain::safe_relative_path(path) {
        return None;
    }
    if !project.join(path).is_file() {
        return None;
    }
    let mut number = || {
        parts
            .next()
            .and_then(|number| number.trim().parse::<usize>().ok())
    };
    let line = number();
    let column = number();
    Some((path.to_string(), line, column))
}

/// One flag of a rule before it is numbered: what it is about, where in the
/// subject it sits, the digest of what was flagged, where to say it is, and
/// why the rule doubts it.
#[derive(Debug, Clone)]
struct Flag {
    subject: String,
    position: (usize, usize),
    subject_digest: String,
    place: String,
    unsure: Option<String>,
}

/// The subject each flag is fingerprinted under. The same rule flagging one
/// subject more than once numbers the repeats in source order (the second is
/// `#2`), so a flag added later in the subject never changes the fingerprint
/// of one before it.
fn numbered(mut flagged: Vec<Flag>) -> Vec<(String, Flag)> {
    flagged.sort_by(|left, right| {
        (&left.subject, left.position).cmp(&(&right.subject, right.position))
    });
    let mut seen = BTreeMap::<String, usize>::new();
    flagged
        .into_iter()
        .map(|flag| {
            let count = seen.entry(flag.subject.clone()).or_insert(0);
            *count += 1;
            let subject = if *count == 1 {
                flag.subject.clone()
            } else {
                format!("{}#{count}", flag.subject)
            };
            (subject, flag)
        })
        .collect()
}

fn fingerprint(rule: &str, repository: &str, subject: &str) -> String {
    digest(format!("{rule}\0{repository}\0{subject}").as_bytes())
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
        let doubt =
            |path: Option<&str>, text: &str| rule.sweep_unsure_for(path, text).map(str::to_owned);
        let mut flagged = Vec::new();
        for (index, failure) in outcome.failures.iter().enumerate() {
            match located(project, &failure.location) {
                Some((path, line, column)) => {
                    let content = flagged_content(project, &path, line);
                    let text =
                        format!("{}\n{}", String::from_utf8_lossy(&content), failure.message);
                    flagged.push(Flag {
                        subject: path.clone(),
                        position: line
                            .map_or((usize::MAX, index), |line| (line, column.unwrap_or(0))),
                        subject_digest: digest(&content),
                        place: path.clone(),
                        unsure: doubt(Some(&path), &text),
                    });
                }
                None => flagged.push(Flag {
                    subject: "repository".into(),
                    position: (usize::MAX, index),
                    subject_digest: digest(
                        format!("{}\0{}", failure.location, failure.message).as_bytes(),
                    ),
                    place: failure.location.clone(),
                    unsure: doubt(None, &format!("{}\n{}", failure.location, failure.message)),
                }),
            }
        }
        if flagged.is_empty() {
            flagged.push(Flag {
                subject: "repository".into(),
                position: (0, 0),
                subject_digest: digest(outcome.summary.as_bytes()),
                place: "the repository".into(),
                unsure: doubt(None, &outcome.summary),
            });
        }
        for (subject, flag) in numbered(flagged) {
            result.push(SweepFinding {
                fingerprint: fingerprint(&outcome.id, &repository, &subject),
                subject_digest: flag.subject_digest,
                class: rule.sweep_class().label(),
                priority: rule.sweep_priority(),
                title: format!("{} ({})", rule.statement, flag.place)
                    .chars()
                    .take(300)
                    .collect(),
                responsibility: outcome.id.clone(),
                unsure: flag.unsure,
            });
        }
    }
    result.sort();
    result.dedup();
    result
}

/// The findings as one JSON array, one finding per line: what a sweep's
/// controller reads as `findings.json` (Messo's `messo-ctl observe` parses
/// the whole document, an object or a list).
pub(crate) fn json_array(findings: &[SweepFinding]) -> String {
    let lines = findings
        .iter()
        .filter_map(|finding| serde_json::to_string(finding).ok())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "[]\n".into()
    } else {
        format!("[\n{}\n]\n", lines.join(",\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_added_repeat_never_changes_an_earlier_fingerprint() {
        let flag = |line: usize, content: &str| Flag {
            subject: "src/app.py".into(),
            position: (line, 1),
            subject_digest: digest(content.as_bytes()),
            place: "src/app.py".into(),
            unsure: None,
        };
        let fingerprints = |flagged: Vec<Flag>| {
            numbered(flagged)
                .into_iter()
                .map(|(subject, _)| fingerprint("rule.print", "example/app", &subject))
                .collect::<Vec<_>>()
        };
        let two = fingerprints(vec![flag(1, "print('x')"), flag(2, "print('x')")]);
        let three = fingerprints(vec![
            flag(3, "print('x')"),
            flag(1, "print('x')"),
            flag(2, "print('x')"),
        ]);
        assert_eq!(two.len(), 2);
        assert_ne!(two[0], two[1], "each repeat is its own finding");
        assert_eq!(
            two[0],
            fingerprint("rule.print", "example/app", "src/app.py"),
            "the first occurrence is fingerprinted by rule, repository and subject"
        );
        assert_eq!(three[..2], two[..], "the earlier fingerprints hold");
        assert_eq!(three.len(), 3);

        // A later flag whose content digest sorts first still numbers after
        // the earlier ones.
        let mut contents = ["print('x')", "print('y')"];
        contents.sort_by_key(|content| std::cmp::Reverse(digest(content.as_bytes())));
        let [earlier, later] = contents;
        let before = fingerprints(vec![flag(1, earlier), flag(2, earlier)]);
        let added = fingerprints(vec![flag(1, earlier), flag(2, earlier), flag(3, later)]);
        assert_eq!(added[..2], before[..]);
    }

    #[test]
    fn findings_carry_priority_always_and_unsure_only_when_set() {
        let finding = |unsure: Option<&str>| SweepFinding {
            fingerprint: "sha256:f".into(),
            subject_digest: "sha256:d".into(),
            class: "discuss",
            priority: 2,
            title: "Print is a smell. (src/app.py)".into(),
            responsibility: "rule.print-advisory".into(),
            unsure: unsure.map(str::to_owned),
        };
        assert_eq!(
            json_array(&[finding(None), finding(Some("borderline"))]),
            "[\n{\"fingerprint\":\"sha256:f\",\"subjectDigest\":\"sha256:d\",\"class\":\"discuss\",\"priority\":2,\"title\":\"Print is a smell. (src/app.py)\",\"responsibility\":\"rule.print-advisory\"},\n{\"fingerprint\":\"sha256:f\",\"subjectDigest\":\"sha256:d\",\"class\":\"discuss\",\"priority\":2,\"title\":\"Print is a smell. (src/app.py)\",\"responsibility\":\"rule.print-advisory\",\"unsure\":\"borderline\"}\n]\n"
        );
    }
}
