//! A check's flags as GitHub workflow commands, so a reviewer sees the rule
//! beside the code it flags: `::error` for a must rule, `::warning` for a
//! should rule, `::notice` for an advisory one. With a base revision only
//! flags on lines the change added or modified are annotated (a flag about a
//! whole file lands on line 1 of a changed file); a flag with no file is
//! annotated without one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

/// The workflow commands for the failed gates in a check's envelope data.
pub fn render(project_root: &Path, data: &Value, base: Option<&str>) -> String {
    let changed = base.and_then(|base| changed_lines(project_root, base));
    let mut out = String::new();
    for gate in data
        .get("gates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if gate.get("state").and_then(Value::as_str) != Some("fail")
            || gate.get("shadow").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let level = match gate.get("strength").and_then(Value::as_str) {
            Some("must") => "error",
            Some("should") => "warning",
            _ => "notice",
        };
        let id = gate.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = gate.get("name").and_then(Value::as_str).unwrap_or_default();
        let title = format!("{id}: {name}");
        let failures = gate
            .get("failures")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if failures.is_empty() {
            let summary = gate
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            command(&mut out, level, None, &title, summary, id);
            continue;
        }
        for failure in failures {
            let location = failure
                .get("location")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let message = failure
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let place = place(project_root, location);
            let shown = match (&place, &changed) {
                (Some((path, line)), Some(changed)) => match changed.get(path) {
                    Some(lines) => line.map_or(true, |line| lines.contains(&line)),
                    None => false,
                },
                _ => true,
            };
            if shown {
                command(
                    &mut out,
                    level,
                    place.map(|(path, line)| (path, line.unwrap_or(1))),
                    &title,
                    message,
                    id,
                );
            }
        }
    }
    out
}

fn command(
    out: &mut String,
    level: &str,
    place: Option<(String, usize)>,
    title: &str,
    message: &str,
    rule: &str,
) {
    let message = format!("{message} (wh explain {rule})");
    let _ = match place {
        Some((path, line)) => writeln!(
            out,
            "::{level} file={},line={line},title={}::{}",
            property(&path),
            property(title),
            data(&message)
        ),
        None => writeln!(
            out,
            "::{level} title={}::{}",
            property(title),
            data(&message)
        ),
    };
}

/// A failure location as a repository file and line, when it names one.
fn place(project_root: &Path, location: &str) -> Option<(String, Option<usize>)> {
    let mut parts = location.splitn(3, ':');
    let path = parts.next()?.trim().trim_start_matches("./");
    if path.is_empty() || path.starts_with('/') || path.contains("..") {
        return None;
    }
    if !project_root.join(path).is_file() {
        return None;
    }
    let line = parts
        .next()
        .and_then(|line| line.trim().parse::<usize>().ok());
    Some((path.to_string(), line))
}

/// The lines each changed file added or modified since `base`, read from a
/// zero-context diff of the working tree.
pub fn changed_lines(project_root: &Path, base: &str) -> Option<BTreeMap<String, BTreeSet<usize>>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args([
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
            base,
            "--",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut changed = BTreeMap::<String, BTreeSet<usize>>::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            current = Some(path.to_string());
            changed.entry(path.to_string()).or_default();
        } else if line.starts_with("+++ ") {
            current = None;
        } else if let (Some(hunk), Some(path)) = (line.strip_prefix("@@ "), current.as_ref()) {
            // @@ -a,b +c,d @@: lines c .. c+d-1 are new (d defaults to 1).
            let Some(added) = hunk.split_whitespace().find(|part| part.starts_with('+')) else {
                continue;
            };
            let mut numbers = added.trim_start_matches('+').split(',');
            let start = numbers.next().and_then(|start| start.parse::<usize>().ok());
            let count = numbers
                .next()
                .map_or(Some(1), |count| count.parse::<usize>().ok());
            if let (Some(start), Some(count)) = (start, count) {
                changed
                    .entry(path.clone())
                    .or_default()
                    .extend(start..start + count);
            }
        }
    }
    Some(changed)
}

/// Escape workflow command data.
fn data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escape a workflow command property.
fn property(text: &str) -> String {
    data(text).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_become_workflow_commands_by_strength_and_place() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::create_dir_all(temp.path().join("src")).expect("src");
        std::fs::write(temp.path().join("src/a.rs"), "a\nb\n").expect("a");
        let data = serde_json::json!({"gates": [
            {"id": "rule.must", "name": "No unwrap, ever", "strength": "must", "state": "fail",
             "failures": [{"location": "src/a.rs:2", "message": "x.unwrap()"}]},
            {"id": "rule.should", "name": "Short files", "strength": "should", "state": "fail",
             "failures": [{"location": "src/a.rs", "message": "100% too long"}]},
            {"id": "rule.repo", "name": "Tests pass", "strength": "must", "state": "fail",
             "failures": [{"location": "exit code 1", "message": "failed"}]},
            {"id": "rule.shadow", "name": "Shadowed", "strength": "must", "state": "fail", "shadow": true,
             "failures": [{"location": "src/a.rs:1", "message": "hidden"}]},
            {"id": "rule.ok", "name": "Fine", "strength": "must", "state": "pass", "failures": []}
        ]});
        assert_eq!(
            render(temp.path(), &data, None),
            "::error file=src/a.rs,line=2,title=rule.must%3A No unwrap%2C ever::x.unwrap() (wh explain rule.must)\n\
             ::warning file=src/a.rs,line=1,title=rule.should%3A Short files::100%25 too long (wh explain rule.should)\n\
             ::error title=rule.repo%3A Tests pass::failed (wh explain rule.repo)\n"
        );
    }
}
