//! Sweep findings: `wh check --sweep` runs only the should rules and turns each flag into one finding for a night sweep's controller
//! to work (Messo's, inside Virgil).
//!
//! A finding is `{fingerprint, subjectDigest, class, priority, title,
//! responsibility, principle?, unsure?, sites?, fix?}`: `priority` (0 most
//! urgent to 4) is the rule's, `principle` is the id of the principle the
//! rule serves when the rulebook names one, `unsure` says why, only where the
//! rule doubts its own flag (the consumer parks such a finding until a person
//! raises it), `sites` lists the flags of a batched rule's finding, and `fix`
//! is the rule's tool's own diff, only where the tool can rewrite files.
//! The fingerprint (rule id, repository and subject) stays the same across
//! runs for the same subject, so a dismissed finding stays quiet until its
//! subject changes; the subject digest changes exactly when the flagged
//! content does. No timestamps, no run ids: the same repository state gives
//! byte-identical findings.
//!
//! A batched rule (`batch: by-file` or `by-crate`) files one finding per
//! file or per crate, its subject the batch, its digest over every site. A
//! change-mode rule files nothing: the sweep counts its flags.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::{Batch, Rule, RuleMode, Strength};
use crate::gates::{GateOutcome, GateState};
use crate::rulebook::Rulebook;

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
    /// The principle the rule serves, when its source names one held in the
    /// rulebook, so the why travels with the finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) principle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unsure: Option<String>,
    /// Every flag a batched rule's finding stands for, in source order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) sites: Vec<Site>,
    /// The tool's own fix, as a unified diff that applies at the repository
    /// root with `git apply` (formatters and lints that can rewrite files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fix: Option<String>,
    /// What the finding is about: a repository path, or `repository`.
    #[serde(skip)]
    pub(crate) subject: String,
}

/// One flag inside a batched finding: where it is and the flagged text (the
/// line, or the tool's message for a flag on a whole file).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Site {
    pub(crate) path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) line: Option<usize>,
    pub(crate) text: String,
}

/// How many times a change-mode rule flagged in a sweep, which files no
/// finding for it. `state` is the gate's (`pass`, `fail` or `unknown`): an
/// unknown count is not zero flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChangeCount {
    pub(crate) rule: String,
    pub(crate) state: GateState,
    pub(crate) count: usize,
}

/// What a sweep reports: its findings, and the counts of its change-mode
/// rules.
#[derive(Debug, Default)]
pub(crate) struct Sweep {
    pub(crate) findings: Vec<SweepFinding>,
    pub(crate) counts: Vec<ChangeCount>,
}

/// Whether a rule's flags are sweep work: below must.
pub(crate) fn sweepable(rule: &Rule) -> bool {
    rule.strength != Strength::Must
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
/// subject it sits, the digest of what was flagged, where to say it is, why
/// the rule doubts it, and the site a batch lists (for a flag on a file).
#[derive(Debug, Clone)]
struct Flag {
    subject: String,
    position: (usize, usize),
    subject_digest: String,
    place: String,
    unsure: Option<String>,
    site: Option<Site>,
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

/// A finding's fingerprint: the rule, the repository and the subject (a
/// path, or `repository`, with `#n` for the same rule's nth flag on it).
pub(crate) fn fingerprint(rule: &str, repository: &str, subject: &str) -> String {
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

/// One finding per flag of every swept rule (one per batch for a batched
/// rule), sorted, and the counts of the change-mode rules.
pub(crate) fn findings(
    project: &Path,
    book: &Rulebook,
    selected: &[SelectedRule],
    outcomes: &[GateOutcome],
) -> Sweep {
    let repository = repository_identity(project);
    let rules = selected
        .iter()
        .map(|selected| (selected.id.as_str(), &selected.rule))
        .collect::<BTreeMap<_, _>>();
    let mut sweep = Sweep::default();
    for outcome in outcomes {
        let Some(rule) = rules.get(outcome.id.as_str()) else {
            continue;
        };
        if !sweepable(rule) {
            continue;
        }
        if rule.mode == RuleMode::Change {
            sweep.counts.push(ChangeCount {
                rule: outcome.id.clone(),
                state: outcome.state,
                count: match outcome.state {
                    GateState::Fail => (outcome.failures.len() + outcome.omitted).max(1),
                    _ => 0,
                },
            });
            continue;
        }
        if outcome.state != GateState::Fail {
            continue;
        }
        let finding = |subject: String, flag: Flag| SweepFinding {
            subject,
            fix: None,
            fingerprint: String::new(),
            subject_digest: flag.subject_digest,
            class: rule.sweep_class().label(),
            priority: rule.sweep_priority(),
            title: format!("{} ({})", rule.statement, flag.place)
                .chars()
                .take(300)
                .collect(),
            responsibility: outcome.id.clone(),
            principle: book
                .principle_for(rule)
                .map(|entry| entry.id.as_str().to_string()),
            unsure: flag.unsure,
            sites: Vec::new(),
        };
        let mut flagged = flags(project, rule, outcome);
        if let Some(batch) = rule.batch {
            let (sited, rest) = flagged.into_iter().partition(|flag| flag.site.is_some());
            flagged = rest;
            for group in batched(project, batch, sited) {
                let mut found = finding(group.subject.clone(), group.flag);
                found.fingerprint = fingerprint(&outcome.id, &repository, &group.subject);
                found.sites = group.sites;
                sweep.findings.push(found);
            }
        }
        for (subject, flag) in numbered(flagged) {
            let mut found = finding(
                subject.split('#').next().unwrap_or_default().to_string(),
                flag,
            );
            found.fingerprint = fingerprint(&outcome.id, &repository, &subject);
            sweep.findings.push(found);
        }
    }
    sweep.findings.sort();
    sweep.findings.dedup();
    sweep
}

/// Every flag of a failed rule, located where the failure names a file.
fn flags(project: &Path, rule: &Rule, outcome: &GateOutcome) -> Vec<Flag> {
    let doubt =
        |path: Option<&str>, text: &str| rule.sweep_unsure_for(path, text).map(str::to_owned);
    let mut flagged = Vec::new();
    for (index, failure) in outcome.failures.iter().enumerate() {
        match located(project, &failure.location) {
            Some((path, line, column)) => {
                let content = flagged_content(project, &path, line);
                let text = format!("{}\n{}", String::from_utf8_lossy(&content), failure.message);
                let site_text = match line {
                    Some(line) if line > 0 => String::from_utf8_lossy(&content).into_owned(),
                    _ => failure.message.clone(),
                };
                flagged.push(Flag {
                    subject: path.clone(),
                    position: line.map_or((usize::MAX, index), |line| (line, column.unwrap_or(0))),
                    subject_digest: digest(&content),
                    place: path.clone(),
                    unsure: doubt(Some(&path), &text),
                    site: Some(Site {
                        path: path.clone(),
                        line: line.filter(|line| *line > 0),
                        text: site_text.chars().take(240).collect(),
                    }),
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
                site: None,
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
            site: None,
        });
    }
    flagged
}

/// The batch a flag on `path` belongs to: the file, or the crate or package
/// holding it (the nearest directory with a `Cargo.toml` or `package.json`;
/// `.` when none above it has one).
pub(crate) fn batch_key(project: &Path, batch: Batch, path: &str) -> String {
    match batch {
        Batch::ByFile => path.to_string(),
        Batch::ByCrate => Path::new(path)
            .ancestors()
            .skip(1)
            .map(|directory| directory.to_string_lossy().into_owned())
            .find(|directory| {
                ["Cargo.toml", "package.json"]
                    .iter()
                    .any(|manifest| project.join(directory).join(manifest).is_file())
            })
            .filter(|directory| !directory.is_empty())
            .unwrap_or_else(|| ".".into()),
    }
}

/// A batched finding's subject: the batch kind and key, and for flags the
/// rule doubts, the reason, so doubtful sites are parked apart from the
/// rest.
pub(crate) fn batch_subject(batch: Batch, key: &str, unsure: Option<&str>) -> String {
    match unsure {
        Some(reason) => format!("{}:{key}:unsure:{reason}", batch.label()),
        None => format!("{}:{key}", batch.label()),
    }
}

/// The batch a batched finding's subject names: its kind and key.
pub(crate) fn batch_of(subject: &str) -> Option<(Batch, &str)> {
    let (kind, rest) = subject.split_once(':')?;
    let batch = [Batch::ByFile, Batch::ByCrate]
        .into_iter()
        .find(|batch| batch.label() == kind)?;
    Some((batch, rest.split(":unsure:").next().unwrap_or(rest)))
}

/// One batch of a batched rule's flags: its subject, a flag standing for it
/// (where to say it is, the digest over its sites and the doubt) and its
/// sites in source order.
struct Batched {
    subject: String,
    flag: Flag,
    sites: Vec<Site>,
}

/// The flags on files grouped into batches.
fn batched(project: &Path, batch: Batch, flagged: Vec<Flag>) -> Vec<Batched> {
    let mut groups = BTreeMap::<(String, Option<String>), Vec<Flag>>::new();
    for flag in flagged {
        let key = batch_key(project, batch, &flag.subject);
        groups
            .entry((key, flag.unsure.clone()))
            .or_default()
            .push(flag);
    }
    groups
        .into_iter()
        .map(|((key, unsure), mut flags)| {
            flags.sort_by(|left, right| {
                (&left.subject, left.position).cmp(&(&right.subject, right.position))
            });
            // The digest covers every site's path and flagged content, in
            // source order, so it changes when a site is fixed, added or
            // edited, and not when an edit elsewhere moves a line.
            let listed = flags
                .iter()
                .map(|flag| format!("{}\0{}\n", flag.subject, flag.subject_digest))
                .collect::<String>();
            let sites = flags
                .into_iter()
                .filter_map(|flag| flag.site)
                .collect::<Vec<_>>();
            let place = match batch {
                Batch::ByCrate if key == "." => {
                    format!("{} site(s) at the repository root", sites.len())
                }
                _ => format!("{} site(s) in {key}", sites.len()),
            };
            let subject = batch_subject(batch, &key, unsure.as_deref());
            Batched {
                flag: Flag {
                    subject: subject.clone(),
                    position: (0, 0),
                    subject_digest: digest(listed.as_bytes()),
                    place,
                    unsure,
                    site: None,
                },
                subject,
                sites,
            }
        })
        .collect()
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
            site: None,
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
            responsibility: "rule.print-doubtful".into(),
            principle: None,
            unsure: unsure.map(str::to_owned),
            sites: Vec::new(),
            fix: None,
            subject: "src/app.py".into(),
        };
        assert_eq!(
            json_array(&[finding(None), finding(Some("borderline"))]),
            "[\n{\"fingerprint\":\"sha256:f\",\"subjectDigest\":\"sha256:d\",\"class\":\"discuss\",\"priority\":2,\"title\":\"Print is a smell. (src/app.py)\",\"responsibility\":\"rule.print-doubtful\"},\n{\"fingerprint\":\"sha256:f\",\"subjectDigest\":\"sha256:d\",\"class\":\"discuss\",\"priority\":2,\"title\":\"Print is a smell. (src/app.py)\",\"responsibility\":\"rule.print-doubtful\",\"unsure\":\"borderline\"}\n]\n"
        );
    }
}
