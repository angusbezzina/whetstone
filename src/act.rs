//! `wh act`: act on a sweep's `findings.json` in a GitHub repository, as
//! Messo does inside Virgil. The reusable Action (`action.yml`) runs it after
//! the sweep. Each finding is worked by its class:
//!
//! - `draft_pr`: one draft pull request per fingerprint. When the finding
//!   carries the tool's own `fix`, the pull request applies it; otherwise it
//!   is an empty commit whose description holds the rule and its
//!   explanation, for an agent to complete.
//! - `confirm_first`: one issue asking a person to approve the change.
//! - `discuss`: one issue labelled for discussion.
//!
//! Fingerprints make every run idempotent. An open item for a fingerprint is
//! updated only when the finding's `subjectDigest` changed; a closed item
//! (the finding was dismissed or fixed) keeps it quiet until the subject
//! changes. A finding marked `unsure` is parked until a person raises it.
//!
//! Issues and pull requests go through `gh`; branches through `git`. Nothing
//! but the findings file and the repository it runs in is read.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::service::{
    CommandService, ExplainRequest, ServiceRequest, ServiceResponse, ServiceState,
};

/// The label every item carries, and the one per class.
pub const LABEL: &str = "whetstone";
const CLASS_LABELS: [(&str, &str, &str); 3] = [
    ("draft_pr", "whetstone:draft-pr", "1d76db"),
    ("confirm_first", "whetstone:confirm-first", "fbca04"),
    ("discuss", "whetstone:discuss", "c5def5"),
];
const LABEL_COLOR: &str = "5319e7";
const MARKER_START: &str = "<!-- whetstone:finding fingerprint=";

#[derive(Debug, Clone)]
pub struct ActRequest {
    pub project_dir: PathBuf,
    pub findings: PathBuf,
    /// The branch draft pull requests target.
    pub base: String,
    pub remote: String,
    /// Report what would be opened or updated, and change nothing.
    pub dry_run: bool,
}

/// One finding as `findings.json` carries it.
#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    pub fingerprint: String,
    #[serde(rename = "subjectDigest")]
    pub subject_digest: String,
    pub class: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    pub title: String,
    pub responsibility: String,
    /// The principle the rule serves, when the rulebook names one.
    #[serde(default)]
    pub principle: Option<String>,
    #[serde(default)]
    pub unsure: Option<String>,
    #[serde(default)]
    pub fix: Option<String>,
}

fn default_priority() -> u8 {
    2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Issue,
    PullRequest,
}

/// An issue or pull request a run opened, found by its marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub number: u64,
    pub open: bool,
    pub fingerprint: String,
    pub subject: String,
}

/// What a run did, by fingerprint.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Report {
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub quiet: Vec<String>,
    pub parked: Vec<String>,
    pub planned: Vec<Value>,
}

/// Where issues and pull requests live.
pub trait Forge {
    /// Every issue and pull request labelled `whetstone`, open or closed.
    fn items(&self) -> Result<Vec<Item>, String>;
    fn ensure_labels(&self) -> Result<(), String>;
    fn create(
        &self,
        kind: Kind,
        title: &str,
        body: &str,
        labels: &[&str],
        branch: Option<(&str, &str)>,
    ) -> Result<(), String>;
    fn edit(&self, kind: Kind, number: u64, body: &str) -> Result<(), String>;
}

/// The marker a body carries, read back from an item.
fn marker(body: &str) -> Option<(String, String)> {
    let rest = &body[body.find(MARKER_START)? + MARKER_START.len()..];
    let (fingerprint, rest) = rest.split_once(" subject=")?;
    let (subject, _) = rest.split_once(" -->")?;
    Some((fingerprint.to_string(), subject.to_string()))
}

fn run(root: &Path, program: &str, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program} could not start: {error}"))?;
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        input
            .write_all(text.as_bytes())
            .map_err(|error| format!("{program} could not read its input: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{program} did not finish: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {} failed: {}",
            args.iter().take(2).copied().collect::<Vec<_>>().join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The GitHub CLI.
pub struct Gh {
    pub root: PathBuf,
}

impl Forge for Gh {
    fn items(&self) -> Result<Vec<Item>, String> {
        let mut found = Vec::new();
        for (kind, noun) in [(Kind::Issue, "issue"), (Kind::PullRequest, "pr")] {
            let listed = run(
                &self.root,
                "gh",
                &[
                    noun,
                    "list",
                    "--state",
                    "all",
                    "--label",
                    LABEL,
                    "--limit",
                    "1000",
                    "--json",
                    "number,state,body",
                ],
                None,
            )?;
            let entries: Vec<Value> = serde_json::from_str(if listed.trim().is_empty() {
                "[]"
            } else {
                &listed
            })
            .map_err(|error| format!("gh {noun} list did not print JSON: {error}"))?;
            for entry in entries {
                let Some((fingerprint, subject)) = marker(entry["body"].as_str().unwrap_or(""))
                else {
                    continue;
                };
                found.push(Item {
                    kind,
                    number: entry["number"].as_u64().unwrap_or_default(),
                    open: entry["state"]
                        .as_str()
                        .is_some_and(|state| state.eq_ignore_ascii_case("open")),
                    fingerprint,
                    subject,
                });
            }
        }
        Ok(found)
    }

    fn ensure_labels(&self) -> Result<(), String> {
        let labels = std::iter::once((LABEL, LABEL_COLOR)).chain(
            CLASS_LABELS
                .iter()
                .map(|(_, label, color)| (*label, *color)),
        );
        for (label, color) in labels {
            run(
                &self.root,
                "gh",
                &["label", "create", label, "--color", color, "--force"],
                None,
            )?;
        }
        Ok(())
    }

    fn create(
        &self,
        kind: Kind,
        title: &str,
        body: &str,
        labels: &[&str],
        branch: Option<(&str, &str)>,
    ) -> Result<(), String> {
        let mut args = match kind {
            Kind::Issue => vec!["issue", "create"],
            Kind::PullRequest => vec!["pr", "create", "--draft"],
        };
        if let Some((base, head)) = branch {
            args.extend(["--base", base, "--head", head]);
        }
        args.extend(["--title", title, "--body-file", "-"]);
        for label in labels {
            args.extend(["--label", label]);
        }
        run(&self.root, "gh", &args, Some(body)).map(|_| ())
    }

    fn edit(&self, kind: Kind, number: u64, body: &str) -> Result<(), String> {
        let noun = match kind {
            Kind::Issue => "issue",
            Kind::PullRequest => "pr",
        };
        let number = number.to_string();
        run(
            &self.root,
            "gh",
            &[noun, "edit", &number, "--body-file", "-"],
            Some(body),
        )
        .map(|_| ())
    }
}

/// Read a findings file: a JSON array (what `wh check --sweep` prints) or an
/// object with a `findings` array.
pub fn load_findings(path: &Path) -> Result<Vec<Finding>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not JSON: {error}", path.display()))?;
    let items = match value {
        Value::Object(mut object) => object.remove("findings").unwrap_or(Value::Null),
        other => other,
    };
    serde_json::from_value(items)
        .map_err(|error| format!("{} is not a findings file: {error}", path.display()))
}

fn class_label(class: &str) -> Option<&'static str> {
    CLASS_LABELS
        .iter()
        .find(|(name, _, _)| *name == class)
        .map(|(_, label, _)| *label)
}

/// `wh explain` for the finding, as a person reads it.
fn explain(root: &Path, finding: &Finding, findings_path: &Path) -> String {
    let response = CommandService.execute(ServiceRequest::Explain(ExplainRequest {
        project_dir: root.to_path_buf(),
        request_id: None,
        target: finding.fingerprint.clone(),
        findings: Some(findings_path.to_path_buf()),
    }));
    response.data["text"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn body(finding: &Finding, explained: &str) -> String {
    let intro = match (finding.class.as_str(), finding.fix.is_some()) {
        ("confirm_first", _) => "Whetstone's sweep found a breach of an accepted rule. The rule says to ask a person first: approve a fix by assigning this issue to an agent, or close it to dismiss the finding (it stays quiet until the flagged code changes).",
        ("discuss", _) => "Whetstone's sweep found a breach of an accepted rule that is for discussion. Decide here whether to fix the code, change the rule (`wh change`), or close this issue to dismiss the finding until the flagged code changes.",
        (_, true) => "Whetstone's sweep found a breach of an accepted rule, and the rule's own tool produced this fix. Review it, mark it ready, or close it to dismiss the finding.",
        (_, false) => "Whetstone's sweep found a breach of an accepted rule that an agent may fix. This draft has no change yet: an agent completes it from the rule and the explanation below, runs `wh check`, and marks it ready.",
    };
    let mut lines = vec![
        intro.to_string(),
        String::new(),
        format!("**Rule:** `{}`", finding.responsibility),
        format!("**Finding:** {}", finding.title),
        format!("**Priority:** {} (0 is most urgent)", finding.priority),
    ];
    if let Some(principle) = &finding.principle {
        lines.push(format!("**Serves:** `{principle}`"));
    }
    lines.push(String::new());
    if !explained.is_empty() {
        lines.extend([
            "<details><summary>wh explain</summary>".to_string(),
            String::new(),
            "```".to_string(),
            explained.to_string(),
            "```".to_string(),
            String::new(),
            "</details>".to_string(),
            String::new(),
        ]);
    }
    lines.extend([
        format!(
            "Explain it locally with `wh explain {}`.",
            finding.fingerprint
        ),
        String::new(),
        format!(
            "{MARKER_START}{} subject={} -->",
            finding.fingerprint, finding.subject_digest
        ),
    ]);
    lines.join("\n")
}

pub fn branch_name(finding: &Finding) -> String {
    let digest = finding
        .fingerprint
        .strip_prefix("sha256:")
        .unwrap_or(&finding.fingerprint);
    format!("whetstone/{}", &digest[..digest.len().min(16)])
}

/// Build the finding's branch from the base, with the fix or an empty
/// commit, push it, and go back to where the checkout was.
fn push_branch(root: &Path, finding: &Finding, base: &str, remote: &str) -> Result<String, String> {
    let git = |args: &[&str], stdin: Option<&str>| run(root, "git", args, stdin);
    let branch = branch_name(finding);
    let original = match git(&["symbolic-ref", "--quiet", "--short", "HEAD"], None) {
        Ok(name) => name.trim().to_string(),
        Err(_) => git(&["rev-parse", "HEAD"], None)?.trim().to_string(),
    };
    git(&["fetch", "--quiet", remote, base], None)?;
    git(
        &[
            "switch",
            "--quiet",
            "--force-create",
            &branch,
            &format!("{remote}/{base}"),
        ],
        None,
    )?;
    let message = format!("whetstone: {}", finding.title)
        .chars()
        .take(200)
        .collect::<String>();
    let built = match &finding.fix {
        Some(fix) => git(&["apply", "--index", "-"], Some(fix))
            .and_then(|_| git(&["commit", "--quiet", "-m", &message], None)),
        None => git(
            &["commit", "--quiet", "--allow-empty", "-m", &message],
            None,
        ),
    }
    .and_then(|_| {
        git(
            &[
                "push",
                "--quiet",
                "--force",
                remote,
                &format!("{branch}:{branch}"),
            ],
            None,
        )
    });
    let restored = git(&["checkout", "--quiet", "--force", &original], None);
    built.and(restored).map(|_| branch)
}

/// Work every finding by its class.
pub fn act(forge: &dyn Forge, request: &ActRequest) -> Result<Report, String> {
    let root = request.project_dir.as_path();
    let findings = load_findings(&request.findings)?;
    let mut known = BTreeMap::<String, Vec<Item>>::new();
    for item in forge.items()? {
        known
            .entry(item.fingerprint.clone())
            .or_default()
            .push(item);
    }
    let mut report = Report::default();
    if !request.dry_run && !findings.is_empty() {
        forge.ensure_labels()?;
    }
    for finding in &findings {
        let fingerprint = finding.fingerprint.clone();
        let Some(class) = class_label(&finding.class).filter(|_| finding.unsure.is_none()) else {
            report.parked.push(fingerprint);
            continue;
        };
        let items = known.get(&fingerprint).cloned().unwrap_or_default();
        let open = items.iter().find(|item| item.open);
        let existing = match open {
            Some(item) if item.subject == finding.subject_digest => {
                report.unchanged.push(fingerprint);
                continue;
            }
            Some(item) => Some(item.clone()),
            // Dismissed or fixed: quiet until the flagged code changes.
            None if items
                .iter()
                .any(|item| item.subject == finding.subject_digest) =>
            {
                report.quiet.push(fingerprint);
                continue;
            }
            None => None,
        };
        let action = if existing.is_some() {
            "update"
        } else {
            "create"
        };
        report
            .planned
            .push(json!({"fingerprint": fingerprint, "class": finding.class, "action": action}));
        if request.dry_run {
            continue;
        }
        let text = body(finding, &explain(root, finding, &request.findings));
        let title = format!("whetstone: {}", finding.title)
            .chars()
            .take(250)
            .collect::<String>();
        let labels = [LABEL, class];
        let kind = if finding.class == "draft_pr" {
            Kind::PullRequest
        } else {
            Kind::Issue
        };
        if kind == Kind::PullRequest {
            let branch = push_branch(root, finding, &request.base, &request.remote)?;
            match &existing {
                None => {
                    forge.create(kind, &title, &text, &labels, Some((&request.base, &branch)))?
                }
                Some(item) => forge.edit(item.kind, item.number, &text)?,
            }
        } else {
            match &existing {
                None => forge.create(kind, &title, &text, &labels, None)?,
                Some(item) => forge.edit(item.kind, item.number, &text)?,
            }
        }
        if existing.is_some() {
            report.updated.push(fingerprint);
        } else {
            report.created.push(fingerprint);
        }
    }
    Ok(report)
}

/// The run as an envelope.
pub fn respond(forge: &dyn Forge, request: &ActRequest) -> ServiceResponse {
    match act(forge, request) {
        Ok(report) => {
            let mut response = ServiceResponse::new_public(
                "act".into(),
                "act",
                if request.dry_run {
                    ServiceState::NeedsDecision
                } else {
                    ServiceState::Success
                },
                format!(
                    "{} created, {} updated, {} unchanged, {} quiet (dismissed until the code changes), {} parked{}.",
                    report.created.len(),
                    report.updated.len(),
                    report.unchanged.len(),
                    report.quiet.len(),
                    report.parked.len(),
                    if request.dry_run {
                        format!("; a dry run: {} would be opened or updated", report.planned.len())
                    } else {
                        String::new()
                    }
                ),
            );
            response.data = json!(report);
            response
        }
        Err(error) => {
            let mut response = ServiceResponse::new_public(
                "act".into(),
                "act",
                ServiceState::Unknown,
                format!("The findings were not worked: {error}"),
            );
            response.permitted_actions =
                vec!["fix what gh or git reported, then run it again; it is idempotent".into()];
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// One issue or pull request: its kind, number, whether it is open, its
    /// body and its labels.
    type Stored = (Kind, u64, bool, String, Vec<String>);

    /// Issues and pull requests in memory.
    #[derive(Default)]
    struct FakeForge {
        items: RefCell<Vec<Stored>>,
    }

    impl Forge for FakeForge {
        fn items(&self) -> Result<Vec<Item>, String> {
            Ok(self
                .items
                .borrow()
                .iter()
                .filter_map(|(kind, number, open, body, _)| {
                    let (fingerprint, subject) = marker(body)?;
                    Some(Item {
                        kind: *kind,
                        number: *number,
                        open: *open,
                        fingerprint,
                        subject,
                    })
                })
                .collect())
        }

        fn ensure_labels(&self) -> Result<(), String> {
            Ok(())
        }

        fn create(
            &self,
            kind: Kind,
            _: &str,
            body: &str,
            labels: &[&str],
            _: Option<(&str, &str)>,
        ) -> Result<(), String> {
            let number = self.items.borrow().len() as u64 + 1;
            self.items.borrow_mut().push((
                kind,
                number,
                true,
                body.into(),
                labels.iter().map(|label| label.to_string()).collect(),
            ));
            Ok(())
        }

        fn edit(&self, kind: Kind, number: u64, body: &str) -> Result<(), String> {
            for item in self.items.borrow_mut().iter_mut() {
                if item.0 == kind && item.1 == number {
                    item.3 = body.into();
                }
            }
            Ok(())
        }
    }

    fn git(root: &Path, args: &[&str]) -> String {
        run(root, "git", args, None).unwrap_or_else(|error| panic!("{error}"))
    }

    fn finding(class: &str, n: u8, subject: &str, fix: Option<&str>) -> Value {
        json!({
            "fingerprint": format!("sha256:{}", format!("{n:x}").repeat(64)),
            "subjectDigest": format!("sha256:{subject}"),
            "class": class,
            "priority": 1,
            "title": format!("Finding {n} (src/a.txt)"),
            "responsibility": "rule.example",
            "fix": fix,
        })
    }

    #[test]
    fn findings_become_work_by_class_once_and_a_dismissed_one_stays_quiet() {
        let temp = tempfile::tempdir().expect("temp");
        let remote = temp.path().join("remote.git");
        let root = temp.path().join("work");
        std::fs::create_dir(&root).expect("work");
        git(temp.path(), &["init", "--quiet", "--bare", "remote.git"]);
        git(&root, &["init", "--quiet", "-b", "main"]);
        git(&root, &["config", "user.name", "Runner"]);
        git(&root, &["config", "user.email", "runner@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir(root.join("src")).expect("src");
        std::fs::write(root.join("src/a.txt"), "one\n").expect("file");
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "start"]);
        git(
            &root,
            &["remote", "add", "origin", remote.to_str().expect("path")],
        );
        git(&root, &["push", "--quiet", "origin", "main"]);
        let fix = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n@@ -1 +1 @@\n-one\n+two\n";
        let findings = temp.path().join("findings.json");
        let write = |items: Vec<Value>| {
            std::fs::write(&findings, serde_json::to_string(&items).expect("json")).expect("write")
        };
        write(vec![
            finding("draft_pr", 1, "a", Some(fix)),
            finding("confirm_first", 2, "b", None),
            finding("discuss", 3, "c", None),
            json!({"fingerprint": "sha256:4", "subjectDigest": "sha256:d", "class": "discuss", "priority": 2, "title": "Doubtful", "responsibility": "rule.example", "unsure": "borderline"}),
        ]);
        let forge = FakeForge::default();
        let request = ActRequest {
            project_dir: root.clone(),
            findings: findings.clone(),
            base: "main".into(),
            remote: "origin".into(),
            dry_run: false,
        };

        let dry = act(
            &forge,
            &ActRequest {
                dry_run: true,
                ..request.clone()
            },
        )
        .expect("dry run");
        assert_eq!(dry.planned.len(), 3);
        assert!(forge.items.borrow().is_empty(), "a dry run opened work");

        let first = act(&forge, &request).expect("first run");
        assert_eq!(first.created.len(), 3, "{first:?}");
        assert_eq!(first.parked, ["sha256:4"]);
        let items = forge.items.borrow().clone();
        assert_eq!(items[0].0, Kind::PullRequest);
        assert!(items[0].4.contains(&"whetstone:draft-pr".to_string()));
        assert_eq!(items[1].0, Kind::Issue);
        assert!(items[1].4.contains(&"whetstone:confirm-first".to_string()));
        assert!(items[2].4.contains(&"whetstone:discuss".to_string()));
        // The draft pull request's branch carries the tool's fix.
        let branch = format!("whetstone/{}", "1".repeat(16));
        let pushed = git(
            temp.path(),
            &[
                "--git-dir",
                remote.to_str().expect("path"),
                "show",
                &format!("{branch}:src/a.txt"),
            ],
        );
        assert_eq!(pushed, "two\n");
        assert_eq!(
            git(&root, &["symbolic-ref", "--short", "HEAD"]).trim(),
            "main"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("src/a.txt")).expect("file"),
            "one\n"
        );

        // A second run changes nothing.
        let second = act(&forge, &request).expect("second run");
        assert_eq!(second.unchanged.len(), 3, "{second:?}");
        assert!(second.created.is_empty() && second.updated.is_empty());

        // A closed issue stays quiet until its subject changes; an open one
        // is updated when it does.
        forge.items.borrow_mut()[1].2 = false;
        write(vec![
            finding("draft_pr", 1, "a", Some(fix)),
            finding("confirm_first", 2, "b", None),
            finding("discuss", 3, "c2", None),
        ]);
        let third = act(&forge, &request).expect("third run");
        assert_eq!(third.quiet, [format!("sha256:{}", "2".repeat(64))]);
        assert_eq!(third.updated, [format!("sha256:{}", "3".repeat(64))]);
        write(vec![finding("confirm_first", 2, "b2", None)]);
        let fourth = act(&forge, &request).expect("fourth run");
        assert_eq!(fourth.created, [format!("sha256:{}", "2".repeat(64))]);
    }

    #[test]
    fn a_body_carries_its_marker_and_the_marker_reads_back() {
        let finding: Finding =
            serde_json::from_value(finding("discuss", 5, "e", None)).expect("finding");
        let text = body(&finding, "Rule rule.example · should · accepted");
        assert!(text.contains("<details><summary>wh explain</summary>"));
        assert_eq!(
            marker(&text),
            Some((finding.fingerprint.clone(), "sha256:e".into()))
        );
        assert_eq!(marker("no marker here"), None);
    }
}
