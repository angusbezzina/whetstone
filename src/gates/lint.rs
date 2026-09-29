//! Lint enforcer: run the linter with its structured reporter and read the
//! exact code from its JSON, instead of searching text output for the code.
//!
//! Supported reporters: ruff (`--output-format json`), clippy / cargo
//! (`--message-format json`), biome (`--reporter json`) and eslint
//! (`--format json`). Another tool falls back to reading its text output and
//! the result says so.

use serde_json::Value;

use crate::proof::ArtifactFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reporter {
    Ruff,
    Cargo,
    Biome,
    Eslint,
    /// No structured reporter is known; the text output is searched.
    Text,
}

impl Reporter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ruff => "ruff json",
            Self::Cargo => "cargo json",
            Self::Biome => "biome json",
            Self::Eslint => "eslint json",
            Self::Text => "text output",
        }
    }
}

/// Pick the reporter for an argv and add the flag that selects it.
pub fn structured(argv: &[String]) -> (Reporter, Vec<String>) {
    let mut argv = argv.to_vec();
    let program = argv
        .first()
        .map(|program| program.rsplit('/').next().unwrap_or(program).to_string())
        .unwrap_or_default();
    let has = |argv: &[String], flag: &str| argv.iter().any(|arg| arg.starts_with(flag));
    let reporter = match program.as_str() {
        "ruff" => Reporter::Ruff,
        "cargo" if argv.iter().any(|arg| arg == "clippy" || arg == "check") => Reporter::Cargo,
        "biome" | "npx" if argv.iter().any(|arg| arg == "biome" || arg == "lint") => {
            if program == "npx" && !argv.iter().any(|arg| arg == "biome") {
                Reporter::Text
            } else {
                Reporter::Biome
            }
        }
        "eslint" => Reporter::Eslint,
        _ => Reporter::Text,
    };
    match reporter {
        Reporter::Ruff if !has(&argv, "--output-format") => {
            argv.push("--output-format".into());
            argv.push("json".into());
        }
        Reporter::Cargo if !has(&argv, "--message-format") => {
            // Cargo flags go before `--`, which separates rustc/clippy flags.
            let at = argv
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or(argv.len());
            argv.insert(at, "--message-format=json".into());
        }
        Reporter::Biome if !has(&argv, "--reporter") => {
            argv.push("--reporter".into());
            argv.push("json".into());
        }
        Reporter::Eslint if !has(&argv, "--format") && !has(&argv, "-f") => {
            argv.push("--format".into());
            argv.push("json".into());
        }
        _ => {}
    }
    (reporter, argv)
}

/// Findings for exactly `code` in a reporter's output, or an error when the
/// output could not be read as that reporter's JSON.
pub fn findings(
    reporter: Reporter,
    output: &str,
    code: &str,
) -> Result<Vec<ArtifactFailure>, String> {
    let mut failures = Vec::new();
    match reporter {
        Reporter::Ruff => {
            let start = output.find('[').ok_or("ruff printed no JSON report")?;
            let end = output.rfind(']').ok_or("ruff printed no JSON report")?;
            let items: Vec<Value> = serde_json::from_str(&output[start..=end])
                .map_err(|error| format!("ruff's JSON report could not be read: {error}"))?;
            for item in items {
                if item.get("code").and_then(Value::as_str) == Some(code) {
                    failures.push(ArtifactFailure {
                        location: format!(
                            "{}:{}",
                            relative(item.get("filename").and_then(Value::as_str).unwrap_or("?")),
                            item.pointer("/location/row")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        ),
                        message: item
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or(code)
                            .to_string(),
                    });
                }
            }
        }
        Reporter::Cargo => {
            let wanted = code.trim_start_matches("clippy::");
            let mut read_any = false;
            for line in output.lines() {
                let Ok(item) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                read_any = true;
                if item.get("reason").and_then(Value::as_str) != Some("compiler-message") {
                    continue;
                }
                let found = item
                    .pointer("/message/code/code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if found.trim_start_matches("clippy::") != wanted {
                    continue;
                }
                let span = item
                    .pointer("/message/spans")
                    .and_then(Value::as_array)
                    .and_then(|spans| {
                        spans.iter().find(|span| {
                            span.get("is_primary").and_then(Value::as_bool) == Some(true)
                        })
                    });
                failures.push(ArtifactFailure {
                    location: format!(
                        "{}:{}",
                        span.and_then(|span| span.get("file_name"))
                            .and_then(Value::as_str)
                            .unwrap_or("?"),
                        span.and_then(|span| span.get("line_start"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    ),
                    message: item
                        .pointer("/message/message")
                        .and_then(Value::as_str)
                        .unwrap_or(code)
                        .to_string(),
                });
            }
            if !read_any {
                return Err("cargo printed no JSON messages".into());
            }
        }
        Reporter::Biome => {
            let start = output.find('{').ok_or("biome printed no JSON report")?;
            let report: Value = serde_json::from_str(&output[start..])
                .map_err(|error| format!("biome's JSON report could not be read: {error}"))?;
            for item in report
                .get("diagnostics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let category = item
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if category != code && !category.ends_with(&format!("/{code}")) {
                    continue;
                }
                failures.push(ArtifactFailure {
                    location: relative(
                        item.pointer("/location/path/file")
                            .and_then(Value::as_str)
                            .unwrap_or("?"),
                    ),
                    message: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or(code)
                        .to_string(),
                });
            }
        }
        Reporter::Eslint => {
            let start = output.find('[').ok_or("eslint printed no JSON report")?;
            let files: Vec<Value> = serde_json::from_str(&output[start..])
                .map_err(|error| format!("eslint's JSON report could not be read: {error}"))?;
            for file in files {
                let path = relative(file.get("filePath").and_then(Value::as_str).unwrap_or("?"));
                for message in file
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if message.get("ruleId").and_then(Value::as_str) == Some(code) {
                        failures.push(ArtifactFailure {
                            location: format!(
                                "{path}:{}",
                                message.get("line").and_then(Value::as_u64).unwrap_or(0)
                            ),
                            message: message
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or(code)
                                .to_string(),
                        });
                    }
                }
            }
        }
        Reporter::Text => {
            for line in output.lines().filter(|line| line.contains(code)).take(20) {
                failures.push(ArtifactFailure {
                    location: "linter output".into(),
                    message: line.trim().chars().take(240).collect(),
                });
            }
        }
    }
    failures.truncate(20);
    Ok(failures)
}

fn relative(path: &str) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            std::path::Path::new(path)
                .strip_prefix(&cwd)
                .ok()
                .map(|relative| relative.display().to_string())
        })
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn reporters_are_selected_and_flags_added_once() {
        assert_eq!(
            structured(&argv("ruff check src")).1,
            argv("ruff check src --output-format json")
        );
        assert_eq!(
            structured(&argv("cargo clippy -- -D warnings")).1,
            argv("cargo clippy --message-format=json -- -D warnings")
        );
        assert_eq!(structured(&argv("npx biome lint .")).0, Reporter::Biome);
        assert_eq!(structured(&argv("./scripts/lint.sh")).0, Reporter::Text);
    }

    #[test]
    fn ruff_findings_match_the_exact_code_only() {
        let output = r#"[{"code":"F401","filename":"a.py","location":{"row":3},"message":"unused"},{"code":"F4011","filename":"b.py","location":{"row":1},"message":"x"}]"#;
        let found = findings(Reporter::Ruff, output, "F401").expect("read");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].location, "a.py:3");
    }

    #[test]
    fn clippy_messages_are_read_from_cargo_json() {
        let output = concat!(
            r#"{"reason":"compiler-artifact"}"#,
            "\n",
            r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::unwrap_used"},"message":"used unwrap","spans":[{"is_primary":true,"file_name":"src/lib.rs","line_start":9}]}}"#
        );
        let found = findings(Reporter::Cargo, output, "unwrap_used").expect("read");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].location, "src/lib.rs:9");
        assert!(findings(Reporter::Cargo, "not json", "x").is_err());
    }
}
