//! Question enforcer: literal yes/no Jev questions, asked through the
//! driver's `ask` command about the smallest unit of change each needs.
//!
//! The kernel builds the unit and redacts it before the driver ever sees it:
//! paths matching a redact glob are replaced whole, pattern matches and
//! built-in secret shapes become stable placeholders, and the receipt keeps a
//! digest of what was replaced, never the text. A local-only rule never
//! calls Jev. No key, no network or no driver is `unavailable`, and a
//! judgment answer never passes a must rule (see `crate::verification`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    ContentDigest, ContextUnit, Enforcer, ExampleVerdict, JudgmentOutcome, Privacy, Rule,
};
use crate::gates::FileSet;

/// At most this many units are asked about per rule per check.
pub const MAX_UNITS: usize = 20;
/// Each unit is cut to this many bytes before it is sent.
pub const MAX_UNIT_BYTES: usize = 8_000;

/// One thing a question is asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unit {
    /// For example `src/app.css:12-30`, `src/app.css` or `commit`.
    pub locator: String,
    pub path: Option<String>,
    pub text: String,
}

/// Text after redaction, and what was replaced (as a digest only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    pub replaced: usize,
    pub digest: Option<ContentDigest>,
}

fn sha(bytes: &[u8]) -> ContentDigest {
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("SHA-256 formatting always satisfies the digest contract")
}

/// Secret shapes that are always redacted, whatever the rule says.
const BUILT_IN_PATTERNS: &[&str] = &[
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
    r"\bgithub_pat_[A-Za-z0-9_]{40,}\b",
    r"\bsk-[A-Za-z0-9_-]{20,}\b",
    r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b",
    r#"(?i)\b(?:api[_-]?key|secret|password|token)\s*[:=]\s*["']?[^\s"']{8,}"#,
];

/// Redact `text` for one unit under the rule's and the repository's privacy.
pub fn redact(privacy: &[&Privacy], path: Option<&str>, text: &str) -> Redacted {
    let mut replacements = Vec::<(String, String)>::new();
    let glob_hit = path.is_some_and(|path| {
        privacy.iter().any(|privacy| {
            privacy.redact_globs.iter().any(|pattern| {
                crate::domain::entry_point_matches(pattern.trim_start_matches("./"), path)
            })
        })
    });
    if glob_hit {
        let placeholder = "[REDACTED:file]".to_string();
        replacements.push((placeholder.clone(), text.to_string()));
        return finish(placeholder, replacements);
    }
    let mut patterns = BUILT_IN_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    for privacy in privacy {
        patterns.extend(privacy.redact_patterns.iter().cloned());
    }
    if let Ok(key) = std::env::var("TYPESAFE_API_KEY") {
        if key.len() >= 8 {
            patterns.push(regex::escape(&key));
        }
    }
    let mut stable = BTreeMap::<String, String>::new();
    let mut result = text.to_string();
    for pattern in patterns {
        let Ok(regex) = Regex::new(&pattern) else {
            continue;
        };
        let mut next = String::with_capacity(result.len());
        let mut last = 0;
        for found in regex.find_iter(&result) {
            let original = found.as_str().to_string();
            let count = stable.len() + 1;
            let placeholder = stable
                .entry(original.clone())
                .or_insert_with(|| format!("[REDACTED:{count}]"))
                .clone();
            next.push_str(&result[last..found.start()]);
            next.push_str(&placeholder);
            last = found.end();
            replacements.push((placeholder, original));
        }
        next.push_str(&result[last..]);
        result = next;
    }
    finish(result, replacements)
}

fn finish(text: String, replacements: Vec<(String, String)>) -> Redacted {
    if replacements.is_empty() {
        return Redacted {
            text,
            replaced: 0,
            digest: None,
        };
    }
    let mut hasher = Sha256::new();
    for (placeholder, original) in &replacements {
        hasher.update(placeholder.as_bytes());
        hasher.update([0]);
        hasher.update(Sha256::digest(original.as_bytes()));
        hasher.update([0]);
    }
    Redacted {
        text,
        replaced: replacements.len(),
        digest: ContentDigest::new(format!("sha256:{:x}", hasher.finalize())).ok(),
    }
}

fn truncate(text: &str) -> String {
    if text.len() <= MAX_UNIT_BYTES {
        return text.to_string();
    }
    let mut end = MAX_UNIT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn git(project_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

/// Hunks of a unified diff, with the file header each belongs to.
pub fn hunks(diff: &str) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut path: Option<String> = None;
    let mut current: Option<(String, u32, String)> = None;
    let flush = |current: &mut Option<(String, u32, String)>, units: &mut Vec<Unit>| {
        if let Some((file, start, text)) = current.take() {
            let lines = text.lines().filter(|line| !line.starts_with('-')).count() as u32;
            units.push(Unit {
                locator: format!("{file}:{start}-{}", start + lines.saturating_sub(2)),
                path: Some(file.clone()),
                text: format!("file: {file}\n{text}"),
            });
        }
    };
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            flush(&mut current, &mut units);
            path = rest
                .strip_prefix("b/")
                .map(str::to_owned)
                .filter(|path| path != "/dev/null");
            continue;
        }
        if line.starts_with("diff --git") || line.starts_with("--- ") || line.starts_with("index ")
        {
            flush(&mut current, &mut units);
            continue;
        }
        if line.starts_with("@@") {
            flush(&mut current, &mut units);
            let start = line
                .split_whitespace()
                .find(|part| part.starts_with('+'))
                .and_then(|part| part[1..].split(',').next())
                .and_then(|number| number.parse::<u32>().ok())
                .unwrap_or(1);
            if let Some(file) = &path {
                current = Some((file.clone(), start, format!("{line}\n")));
            }
            continue;
        }
        if let Some((_, _, text)) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    flush(&mut current, &mut units);
    units
}

/// What a check asks about: the change against `base`, or the staged index.
pub struct UnitSource<'a> {
    pub project_root: &'a Path,
    pub files: &'a FileSet,
    pub staged: bool,
    pub base: Option<&'a str>,
    /// Paths the change touched.
    pub changed: &'a [String],
}

fn commit_message(source: &UnitSource<'_>) -> String {
    let message = if source.staged {
        std::fs::read_to_string(source.project_root.join(".git/COMMIT_EDITMSG")).unwrap_or_default()
    } else if let Some(base) = source.base {
        git(
            source.project_root,
            &["log", "--format=%B", &format!("{base}..HEAD")],
        )
        .unwrap_or_default()
    } else {
        git(source.project_root, &["log", "-1", "--format=%B"]).unwrap_or_default()
    };
    let message = message.trim();
    if message.is_empty() {
        "(no commit message yet)".into()
    } else {
        message.chars().take(2_000).collect()
    }
}

/// The units a question rule asks about, in path order, bounded.
pub fn units(rule: &Rule, unit: ContextUnit, source: &UnitSource<'_>) -> Vec<Unit> {
    // Whetstone's own wiring is never sent: it is not the project's change.
    let in_scope = |path: &str| rule.applies_to(path) && !crate::skill::is_generated(path);
    let mut result = match unit {
        ContextUnit::Hunk => {
            let diff = if source.staged {
                git(
                    source.project_root,
                    &["diff", "--cached", "--no-color", "-U3", "--no-renames"],
                )
            } else if let Some(base) = source.base {
                git(
                    source.project_root,
                    &["diff", "--no-color", "-U3", "--no-renames", base],
                )
            } else {
                git(
                    source.project_root,
                    &["diff", "HEAD", "--no-color", "-U3", "--no-renames"],
                )
            }
            .unwrap_or_default();
            hunks(&diff)
                .into_iter()
                .filter(|unit| unit.path.as_deref().is_some_and(in_scope))
                .collect()
        }
        ContextUnit::File => source
            .files
            .iter()
            .filter(|(path, _)| {
                in_scope(path) && source.changed.iter().any(|changed| changed == path)
            })
            .filter_map(|(path, bytes)| {
                Some(Unit {
                    locator: path.to_string(),
                    path: Some(path.to_string()),
                    text: format!("file: {path}\n{}", std::str::from_utf8(bytes).ok()?),
                })
            })
            .collect(),
        ContextUnit::AddedFile => {
            let listed = if source.staged {
                git(
                    source.project_root,
                    &["diff", "--cached", "--name-only", "--diff-filter=A", "-z"],
                )
            } else if let Some(base) = source.base {
                git(
                    source.project_root,
                    &["diff", "--name-only", "--diff-filter=A", "-z", base],
                )
            } else {
                git(
                    source.project_root,
                    &["ls-files", "--others", "--exclude-standard", "-z"],
                )
            }
            .unwrap_or_default();
            let message = commit_message(source);
            listed
                .split('\0')
                .filter(|path| !path.is_empty() && in_scope(path))
                .filter_map(|path| {
                    let text =
                        source.files.text(path).map(str::to_owned).or_else(|| {
                            std::fs::read_to_string(source.project_root.join(path)).ok()
                        })?;
                    Some(Unit {
                        locator: path.to_string(),
                        path: Some(path.to_string()),
                        text: format!("commit: {message}\nadded file {path}\n{text}"),
                    })
                })
                .collect()
        }
        ContextUnit::Commit => {
            let touched = source
                .changed
                .iter()
                .filter(|path| in_scope(path))
                .cloned()
                .collect::<Vec<_>>();
            if touched.is_empty() {
                Vec::new()
            } else {
                vec![Unit {
                    locator: "commit".into(),
                    path: None,
                    text: format!(
                        "commit: {}\nchanged: {}",
                        commit_message(source),
                        touched.join(", ")
                    ),
                }]
            }
        }
    };
    result.truncate(MAX_UNITS);
    for unit in &mut result {
        unit.text = truncate(&unit.text);
    }
    result
}

/// The exact question Jev receives, with the rule's labelled examples as
/// its criteria (yes means the rule is broken).
pub fn question_request(rule: &Rule, text: &str) -> Option<Value> {
    let Enforcer::Question {
        question,
        yes_means,
        no_means,
        model,
        ..
    } = &rule.enforcer
    else {
        return None;
    };
    let examples = |verdict| {
        rule.examples
            .iter()
            .filter(|example| example.expected == verdict)
            .take(5)
            .map(|example| example.input.chars().take(600).collect::<String>())
            .collect::<Vec<_>>()
    };
    let flag_examples = examples(ExampleVerdict::Flag);
    let pass_examples = examples(ExampleVerdict::Pass);
    let criteria = (yes_means.is_some()
        || no_means.is_some()
        || !flag_examples.is_empty()
        || !pass_examples.is_empty())
    .then(|| {
        json!({
            "true": {"what": yes_means.clone().unwrap_or_else(|| "The rule is broken".into()), "examples": flag_examples},
            "false": {"what": no_means.clone().unwrap_or_else(|| "The rule holds".into()), "examples": pass_examples},
        })
    });
    Some(json!({
        "question": question,
        "criteria": criteria,
        "model": model,
        "state": text,
    }))
}

/// Digest of everything but the unit: the question, its criteria and model.
pub fn question_digest(request: &Value) -> ContentDigest {
    let mut fixed = request.clone();
    if let Some(object) = fixed.as_object_mut() {
        object.remove("state");
    }
    sha(&serde_json::to_vec(&fixed).unwrap_or_default())
}

pub fn input_digest(text: &str) -> ContentDigest {
    sha(text.as_bytes())
}

/// Turn a probability of yes into an outcome under the rule's bars.
/// Confidence is the distance from even odds: |2p − 1|.
pub fn outcome(
    probability_bp: u16,
    threshold_bp: u16,
    confidence_bar_bp: u16,
) -> (JudgmentOutcome, u16) {
    let probability = i32::from(probability_bp.min(10_000));
    let confidence = (2 * probability - 10_000).unsigned_abs() as u16;
    let outcome = if confidence < confidence_bar_bp {
        JudgmentOutcome::LowConfidence
    } else if probability_bp >= threshold_bp {
        JudgmentOutcome::Flag
    } else {
        JudgmentOutcome::Clear
    };
    (outcome, confidence)
}

/// One answer from the driver, or why there is none.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Answered {
        probability_bp: u16,
        model: String,
        request_id: Option<String>,
        input_tokens: Option<u64>,
    },
    Unavailable(String),
}

/// Ask through the driver. The request file sits in the private run
/// directory; the key reaches only the driver process, through its
/// environment.
pub fn ask(project_root: &Path, run_dir: &Path, name: &str, request: &Value) -> Answer {
    if std::env::var("WHETSTONE_JEV_OFFLINE").is_ok_and(|value| value == "1") {
        return Answer::Unavailable(
            "Jev is switched off for this run (WHETSTONE_JEV_OFFLINE=1).".into(),
        );
    }
    let Some(driver) = crate::gates::driver_path(project_root) else {
        return Answer::Unavailable(format!(
            "No verification driver exists at {}; run wh init --action wire.",
            crate::gates::DRIVER_RELATIVE
        ));
    };
    let driver_text = std::fs::read_to_string(project_root.join(&driver)).unwrap_or_default();
    if !driver_text.contains("case \"ask\"") {
        return Answer::Unavailable(
            "The team's driver has no ask command yet; add it with wh init --action wire --regenerate-driver (or copy the ask section from the scaffold).".into(),
        );
    }
    let node = match crate::gates::resolve_program(project_root, "node") {
        Ok(node) => node,
        Err(error) => return Answer::Unavailable(error),
    };
    if std::fs::create_dir_all(run_dir).is_err() {
        return Answer::Unavailable("The run directory could not be created.".into());
    }
    let request_file = run_dir.join(format!("{name}.ask.json"));
    if std::fs::write(
        &request_file,
        serde_json::to_vec(request).unwrap_or_default(),
    )
    .is_err()
    {
        return Answer::Unavailable("The question could not be written for the driver.".into());
    }
    let mut extra = Vec::new();
    for key in ["TYPESAFE_API_KEY", "TYPESAFE_BASE_URL"] {
        if let Ok(value) = std::env::var(key) {
            extra.push((key, value));
        }
    }
    let environment = crate::gates::gate_environment(&extra);
    let result = crate::execution::run_bounded(
        &node,
        &[
            driver,
            "ask".into(),
            "--request-file".into(),
            request_file.display().to_string(),
            "--json".into(),
        ],
        project_root,
        &environment,
        Duration::from_secs(45),
        256 * 1024,
        256 * 1024,
    );
    let _ = std::fs::remove_file(&request_file);
    let result = match result {
        Ok(result) => result,
        Err(error) => return Answer::Unavailable(format!("The driver could not start: {error}")),
    };
    let report = result
        .stdout
        .text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok());
    let Some(report) = report else {
        return Answer::Unavailable("The driver printed no answer.".into());
    };
    match report.get("probability").and_then(Value::as_f64) {
        Some(probability)
            if report.get("ok") == Some(&Value::Bool(true))
                && (0.0..=1.0).contains(&probability) =>
        {
            Answer::Answered {
                probability_bp: (probability * 10_000.0).round() as u16,
                model: report
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                request_id: report
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input_tokens: report.get("input_tokens").and_then(Value::as_u64),
            }
        }
        _ => Answer::Unavailable(
            report
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("Jev did not answer.")
                .chars()
                .take(300)
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_globs_patterns_and_secrets_never_survive() {
        let privacy = Privacy {
            redact_globs: vec!["secrets/**".into()],
            redact_patterns: vec![r"customer-\d+".into()],
            local_only: false,
        };
        let whole = redact(
            &[&privacy],
            Some("secrets/prod.env"),
            "DATABASE=postgres://x",
        );
        assert_eq!(whole.text, "[REDACTED:file]");
        assert!(whole.digest.is_some());
        let text = "charge customer-42 then customer-42 again with ghp_abcdefghijklmnopqrstuvwxyz0123456789AB";
        let redacted = redact(&[&privacy], Some("src/billing.rs"), text);
        assert!(!redacted.text.contains("customer-42"));
        assert!(!redacted.text.contains("ghp_"));
        // Built-in secret shapes run first, so the token is 1 and the
        // repeated customer id is the same placeholder both times.
        assert_eq!(
            redacted.text.matches("[REDACTED:2]").count(),
            2,
            "{}",
            redacted.text
        );
        assert_eq!(redacted.replaced, 3);
        let clean = redact(&[&privacy], Some("src/lib.rs"), "fn main() {}");
        assert_eq!(clean.replaced, 0);
        assert!(clean.digest.is_none());
    }

    #[test]
    fn outcomes_follow_threshold_and_confidence_bar() {
        assert_eq!(outcome(9_500, 5_000, 4_000).0, JudgmentOutcome::Flag);
        assert_eq!(outcome(500, 5_000, 4_000).0, JudgmentOutcome::Clear);
        assert_eq!(
            outcome(5_500, 5_000, 4_000).0,
            JudgmentOutcome::LowConfidence
        );
        assert_eq!(outcome(10_000, 5_000, 4_000).1, 10_000);
    }

    #[test]
    fn hunks_carry_their_file_and_new_line_range() {
        let diff = "diff --git a/src/a.css b/src/a.css\n--- a/src/a.css\n+++ b/src/a.css\n@@ -1,2 +1,3 @@\n .x {\n+  color: red;\n }\n@@ -10 +11,2 @@\n-a\n+b\n+c\n";
        let units = hunks(diff);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].path.as_deref(), Some("src/a.css"));
        assert!(units[0].locator.starts_with("src/a.css:1-"));
        assert!(units[1]
            .text
            .starts_with("file: src/a.css\n@@ -10 +11,2 @@"));
    }

    #[test]
    fn the_question_digest_ignores_the_unit_and_the_input_digest_binds_it() {
        let rule: Rule = serde_json::from_value(json!({
            "schema": "whetstone.rule.v2",
            "statement": "No unrequested tests",
            "rationale": "Maintenance",
            "strength": "should",
            "enforcer": {"kind": "question", "question": "Does this add an unrequested test?"},
            "examples": [{"input": "adds test", "expected": "flag", "reason": "r"}],
            "source": {"kind": "owner"}
        }))
        .expect("rule");
        let one = question_request(&rule, "unit one").expect("request");
        let two = question_request(&rule, "unit two").expect("request");
        assert_eq!(question_digest(&one), question_digest(&two));
        assert_ne!(input_digest("unit one"), input_digest("unit two"));
        assert_eq!(one["criteria"]["true"]["examples"][0], "adds test");
    }
}
