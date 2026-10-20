//! The decision log: the Git commits that touched the rulebook, and the
//! decision trail drawn from them in pstack show-me-your-work's TSV (ts,
//! phase, decision, why, evidence, result), which pstack's audit flow reads
//! unchanged.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::rulebook::RULEBOOK_PATH;

/// One commit that touched the rulebook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulebookCommit {
    pub commit: String,
    pub author: String,
    /// The author date in UTC, RFC 3339 to the second.
    pub date: String,
    pub subject: String,
    /// The message after the subject, trimmed; empty when there is none.
    pub body: String,
}

/// Every commit that touched the rulebook, newest first. A repository with
/// no commit yet has an empty log.
pub fn rulebook_log(project_root: &Path) -> Result<Vec<RulebookCommit>, String> {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("TZ", "UTC")
            .output()
            .map_err(|error| format!("git could not start: {error}"))
    };
    if !git(&["rev-parse", "--verify", "--quiet", "HEAD"])?
        .status
        .success()
    {
        return Ok(Vec::new());
    }
    let output = git(&[
        "log",
        "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
        "--format=%H%x1f%an%x1f%ad%x1f%s%x1f%b%x1e",
        "--",
        RULEBOOK_PATH,
    ])?;
    if !output.status.success() {
        return Err(format!(
            "git log of {RULEBOOK_PATH} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\u{1e}')
        .filter_map(|entry| {
            let mut fields = entry.trim_start_matches('\n').splitn(5, '\u{1f}');
            Some(RulebookCommit {
                commit: fields.next().filter(|commit| !commit.is_empty())?.into(),
                author: fields.next()?.into(),
                date: fields.next()?.into(),
                subject: fields.next()?.into(),
                body: fields.next()?.trim().into(),
            })
        })
        .collect())
}

/// The rulebook file's text at each of `commits`, in the same order, read
/// with one `git cat-file --batch`; `None` where the file is absent there.
pub fn rulebook_texts(
    project_root: &Path,
    commits: &[&str],
) -> Result<Vec<Option<String>>, String> {
    use std::io::Write as _;
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["cat-file", "--batch"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("git could not start: {error}"))?;
    let request = commits
        .iter()
        .map(|commit| format!("{commit}:{RULEBOOK_PATH}\n"))
        .collect::<String>();
    let mut stdin = child.stdin.take().ok_or("git cat-file has no stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(request.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| format!("git cat-file failed: {error}"))?;
    let _ = writer.join();
    let bytes = output.stdout;
    let mut texts = Vec::with_capacity(commits.len());
    let mut at = 0;
    while texts.len() < commits.len() && at < bytes.len() {
        let end = bytes[at..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| at + offset)
            .ok_or("git cat-file output ended early")?;
        let header = String::from_utf8_lossy(&bytes[at..end]).into_owned();
        at = end + 1;
        let mut fields = header.split(' ');
        match (fields.next(), fields.next(), fields.next()) {
            (Some(_), Some("blob"), Some(size)) => {
                let size = size
                    .parse::<usize>()
                    .map_err(|_| format!("git cat-file sent a bad size: {header}"))?;
                let body = bytes
                    .get(at..at + size)
                    .ok_or("git cat-file output ended early")?;
                texts.push(Some(String::from_utf8_lossy(body).into_owned()));
                at += size + 1;
            }
            _ => texts.push(None),
        }
    }
    texts.resize(commits.len(), None);
    Ok(texts)
}

/// pstack show-me-your-work's header row, verbatim.
pub const TRAIL_HEADER: &str = "ts\tphase\tdecision\twhy\tevidence\tresult";

/// The decision trail, oldest first: one row per commit that touched the
/// rulebook.
pub fn decision_trail(log: &[RulebookCommit]) -> String {
    let mut out = String::from(TRAIL_HEADER);
    out.push('\n');
    for commit in log.iter().rev() {
        let why = commit.body.split("\n\n").next().unwrap_or_default();
        let cells = [
            commit.date.as_str(),
            "rulebook",
            commit.subject.as_str(),
            why,
            &format!("git:{} {RULEBOOK_PATH}", commit.commit),
            &format!("committed by {}", commit.author),
        ];
        out.push_str(
            &cells
                .iter()
                .map(|cell| trail_cell(cell))
                .collect::<Vec<_>>()
                .join("\t"),
        );
        out.push('\n');
    }
    out
}

/// One single-line TSV cell: tabs and newlines become spaces, and a leading
/// `=`, `+`, `-` or `@` is quoted so spreadsheets never evaluate it.
fn trail_cell(value: &str) -> String {
    let flat = value
        .chars()
        .map(|character| {
            if matches!(character, '\t' | '\n' | '\r') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let flat = flat.trim().to_string();
    if flat.starts_with(['=', '+', '-', '@']) {
        format!("'{flat}")
    } else if flat.is_empty() {
        "none".into()
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trail_is_oldest_first_with_flat_cells() {
        let log = [
            RulebookCommit {
                commit: "b".repeat(40),
                author: "Owner".into(),
                date: "2026-10-18T10:00:00Z".into(),
                subject: "Retire rule.x".into(),
                body: "rule.x: =superseded\tby rule.y\n\nmore".into(),
            },
            RulebookCommit {
                commit: "a".repeat(40),
                author: "Owner".into(),
                date: "2026-10-17T10:00:00Z".into(),
                subject: "Add rule.x".into(),
                body: String::new(),
            },
        ];
        let trail = decision_trail(&log);
        let rows = trail.lines().collect::<Vec<_>>();
        assert_eq!(rows[0], TRAIL_HEADER);
        assert_eq!(
            rows[1],
            format!(
                "2026-10-17T10:00:00Z\trulebook\tAdd rule.x\tnone\tgit:{} {RULEBOOK_PATH}\tcommitted by Owner",
                "a".repeat(40)
            )
        );
        assert!(rows[2].contains("\trule.x: =superseded by rule.y\t"));
        assert_eq!(trail_cell("=1+1"), "'=1+1");
    }
}
