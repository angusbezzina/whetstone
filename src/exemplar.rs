//! Exemplar codebases: read a repository the owner admires (a local path or
//! a Git URL, shallow-cloned with hooks off) without executing anything, and
//! turn what it demonstrably does into rule drafts with provenance.
//!
//! This is the deterministic half. The skill reads the exemplar further and
//! proposes judgment rules through `wh change --kind rule --source ...`; every
//! draft names the exact files it came from, and nothing is in force until the
//! owner accepts it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::{
    ContentDigest, Enforcer, EvidenceRef, ExampleVerdict, Privacy, Rule, RuleExample, RuleSource,
    RuleSourceKind, Strength, RULE_SCHEMA_V2,
};

/// What was read, and from where.
#[derive(Debug, Clone, Serialize)]
pub struct Exemplar {
    pub locator: String,
    /// The commit read, for a Git checkout.
    pub commit: Option<String>,
    pub root: PathBuf,
    pub files_read: Vec<EvidenceRef>,
    pub languages: Vec<String>,
}

/// A rule the exemplar demonstrates, as a draft.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub id: String,
    pub why: String,
    pub rule: Rule,
}

fn digest(bytes: &[u8]) -> Option<ContentDigest> {
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))).ok()
}

fn is_git_url(locator: &str) -> bool {
    locator.starts_with("https://")
        || locator.starts_with("git@")
        || locator.starts_with("ssh://")
        || locator.ends_with(".git")
}

/// Open an exemplar: a local directory as it is, or a Git URL cloned
/// shallowly into `cache` with hooks and filters disabled.
pub fn open(locator: &str, cache: &Path) -> Result<Exemplar, String> {
    let (root, commit) = if is_git_url(locator) {
        if locator.contains(char::is_whitespace) || locator.starts_with('-') {
            return Err("The exemplar URL is not a plain Git URL.".into());
        }
        let target = cache.join(&format!("{:x}", Sha256::digest(locator.as_bytes()))[..16]);
        if !target.join(".git").is_dir() {
            fs::create_dir_all(cache).map_err(|error| error.to_string())?;
            let status = Command::new("git")
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "protocol.file.allow=never",
                    "clone",
                    "--depth",
                    "1",
                    "--no-tags",
                    "--quiet",
                    "--",
                    locator,
                ])
                .arg(&target)
                .env("GIT_TERMINAL_PROMPT", "0")
                .status()
                .map_err(|error| format!("git is unavailable: {error}"))?;
            if !status.success() {
                return Err(format!(
                    "{locator} could not be cloned; check the URL and your access."
                ));
            }
        }
        let head = Command::new("git")
            .arg("-C")
            .arg(&target)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
        (target, head)
    } else {
        let root = PathBuf::from(locator)
            .canonicalize()
            .map_err(|error| format!("{locator} is not a readable directory: {error}"))?;
        if !root.is_dir() {
            return Err(format!("{locator} is not a directory."));
        }
        let head = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "--verify", "-q", "HEAD"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
        (root, head)
    };
    let mut languages = std::collections::BTreeSet::new();
    for entry in walkdir::WalkDir::new(&root)
        .max_depth(6)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            entry.depth() == 0
                || !matches!(
                    name.as_ref(),
                    ".git" | "node_modules" | "target" | "dist" | "build" | ".venv"
                )
        })
        .filter_map(Result::ok)
        .take(20_000)
    {
        if let Some(language) = crate::types::source_language_for_path(entry.path()) {
            languages.insert(language.to_string());
        }
    }
    Ok(Exemplar {
        locator: locator.into(),
        commit,
        root,
        files_read: Vec::new(),
        languages: languages.into_iter().collect(),
    })
}

fn read(exemplar: &mut Exemplar, relative: &str) -> Option<String> {
    let path = exemplar.root.join(relative);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    exemplar.files_read.push(EvidenceRef {
        system: "exemplar".into(),
        locator: format!(
            "{}@{}:{relative}",
            exemplar.locator,
            exemplar.commit.as_deref().unwrap_or("working-tree")
        ),
        digest: digest(&bytes),
    });
    String::from_utf8(bytes).ok()
}

#[allow(clippy::too_many_arguments)]
fn rule(
    exemplar: &Exemplar,
    evidence: &[EvidenceRef],
    statement: &str,
    rationale: &str,
    strength: Strength,
    enforcer: Enforcer,
    examples: Vec<RuleExample>,
    paths: Vec<String>,
) -> Rule {
    Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: statement.into(),
        rationale: rationale.into(),
        strength,
        enforcer,
        examples,
        source: RuleSource {
            kind: RuleSourceKind::Exemplar,
            reference: Some(format!(
                "{}@{}",
                exemplar.locator,
                exemplar.commit.as_deref().unwrap_or("working-tree")
            )),
            provenance: evidence.to_vec(),
        },
        paths,
        hand_raise: Vec::new(),
        privacy: Privacy::default(),
    }
}

fn example(input: &str, expected: ExampleVerdict, reason: &str, path: &str) -> RuleExample {
    RuleExample {
        input: input.into(),
        expected,
        reason: reason.into(),
        path: Some(path.into()),
    }
}

/// Rules the exemplar demonstrably holds itself to, detected from its
/// configuration and source, each bound to the files it came from.
pub fn propose(exemplar: &mut Exemplar) -> Vec<Proposal> {
    let mut proposals = Vec::new();
    // Clippy lints the exemplar denies.
    if let Some(cargo) = read(exemplar, "Cargo.toml") {
        let evidence = exemplar
            .files_read
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if let Ok(value) = cargo.parse::<toml::Value>() {
            let lints = value
                .get("lints")
                .and_then(|lints| lints.get("clippy"))
                .and_then(toml::Value::as_table)
                .cloned()
                .unwrap_or_default();
            for (lint, level) in lints {
                let level = level.as_str().map(str::to_owned).or_else(|| {
                    level
                        .get("level")
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned)
                });
                if !matches!(level.as_deref(), Some("deny" | "forbid")) {
                    continue;
                }
                proposals.push(Proposal {
                    id: format!("rule.exemplar-clippy-{}", lint.replace('_', "-")),
                    why: format!("The exemplar denies clippy::{lint} in Cargo.toml."),
                    rule: rule(
                        exemplar,
                        &evidence,
                        &format!("Keep clippy::{lint} clean."),
                        &format!("The exemplar treats clippy::{lint} as an error."),
                        Strength::Must,
                        Enforcer::Lint {
                            tool: "cargo clippy --all-targets -- -D warnings".into(),
                            code: lint.clone(),
                        },
                        Vec::new(),
                        vec!["src/**".into()],
                    ),
                });
            }
        }
        // `#![forbid(unsafe_code)]` in the crate root.
        let root = read(exemplar, "src/lib.rs").or_else(|| read(exemplar, "src/main.rs"));
        if root.is_some_and(|text| text.contains("#![forbid(unsafe_code)]")) {
            let evidence = exemplar
                .files_read
                .last()
                .cloned()
                .into_iter()
                .collect::<Vec<_>>();
            proposals.push(Proposal {
                id: "rule.exemplar-no-unsafe".into(),
                why: "The exemplar forbids unsafe code at its crate root.".into(),
                rule: rule(
                    exemplar,
                    &evidence,
                    "No unsafe blocks.",
                    "The exemplar forbids unsafe code; memory safety is not traded for convenience.",
                    Strength::Must,
                    Enforcer::Ast {
                        query: "(unsafe_block) @match".into(),
                        language: Some("rust".into()),
                    },
                    vec![
                        example("fn f(p: *const u8) -> u8 { unsafe { *p } }", ExampleVerdict::Flag, "An unsafe block.", "src/lib.rs"),
                        example("fn f(v: &[u8]) -> u8 { v[0] }", ExampleVerdict::Pass, "Safe indexing.", "src/lib.rs"),
                    ],
                    vec!["src/**".into()],
                ),
            });
        }
    }
    // TypeScript strict mode.
    if let Some(tsconfig) = read(exemplar, "tsconfig.json") {
        let evidence = exemplar
            .files_read
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if tsconfig.replace(' ', "").contains("\"strict\":true") {
            proposals.push(Proposal {
                id: "rule.exemplar-typescript-strict".into(),
                why: "The exemplar compiles TypeScript with strict on.".into(),
                rule: rule(
                    exemplar,
                    &evidence,
                    "TypeScript type-checks under strict mode.",
                    "The exemplar keeps the compiler strict so illegal states stay unrepresentable.",
                    Strength::Must,
                    Enforcer::Validator {
                        command: "npx tsc --noEmit".into(),
                    },
                    Vec::new(),
                    Vec::new(),
                ),
            });
        }
    }
    // A design-token file: a stylesheet that defines many custom properties.
    for candidate in [
        "src/tokens.css",
        "src/styles/tokens.css",
        "styles/tokens.css",
        "tokens.css",
        "src/theme.css",
        "app/globals.css",
        "src/index.css",
    ] {
        let Some(text) = read(exemplar, candidate) else {
            continue;
        };
        let evidence = exemplar
            .files_read
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if crate::gates::tokens::Tokens::parse(candidate, &text)
            .is_ok_and(|tokens| tokens.len() >= 8)
        {
            proposals.push(Proposal {
                id: "rule.exemplar-design-tokens".into(),
                why: format!(
                    "The exemplar defines its visual vocabulary as tokens in {candidate}."
                ),
                rule: rule(
                    exemplar,
                    &evidence,
                    "Use design tokens, not literal colours or spacing.",
                    "The exemplar keeps one token vocabulary so the design system never drifts.",
                    Strength::Should,
                    // The same relative path in this repository; the owner
                    // edits it before accepting when the token file differs.
                    Enforcer::DesignTokens {
                        tokens: candidate.into(),
                        stylesheets: Vec::new(),
                        sizes: true,
                    },
                    vec![
                        example(
                            ".b { color: #ff0000; }",
                            ExampleVerdict::Flag,
                            "A literal colour.",
                            "src/app.css",
                        ),
                        example(
                            ".b { color: var(--accent); }",
                            ExampleVerdict::Pass,
                            "A token.",
                            "src/app.css",
                        ),
                    ],
                    Vec::new(),
                ),
            });
            break;
        }
    }
    // A formatter the exemplar enforces.
    for (file, command) in [
        ("rustfmt.toml", "cargo fmt --check"),
        (".rustfmt.toml", "cargo fmt --check"),
        ("biome.json", "npx biome format ."),
        (".prettierrc", "npx prettier --check ."),
    ] {
        if read(exemplar, file).is_some() {
            let evidence = exemplar
                .files_read
                .last()
                .cloned()
                .into_iter()
                .collect::<Vec<_>>();
            proposals.push(Proposal {
                id: "rule.exemplar-formatted".into(),
                why: format!("The exemplar configures its formatter in {file}."),
                rule: rule(
                    exemplar,
                    &evidence,
                    "Code is formatted by the project's formatter.",
                    "The exemplar never lets formatting reach review.",
                    Strength::Should,
                    Enforcer::Formatter {
                        tool: command.into(),
                    },
                    Vec::new(),
                    Vec::new(),
                ),
            });
            break;
        }
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_exemplar_yields_drafts_bound_to_the_files_read() {
        let temp = tempfile::tempdir().expect("temp");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[lints.clippy]\nunwrap_used = \"deny\"\ntodo = \"warn\"\n",
        )
        .expect("cargo");
        fs::write(
            temp.path().join("src/lib.rs"),
            "#![forbid(unsafe_code)]\npub fn f() {}\n",
        )
        .expect("lib");
        let mut exemplar = open(
            temp.path().to_str().expect("path"),
            &temp.path().join("cache"),
        )
        .expect("open");
        let proposals = propose(&mut exemplar);
        let ids = proposals
            .iter()
            .map(|proposal| proposal.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"rule.exemplar-clippy-unwrap-used"), "{ids:?}");
        assert!(ids.contains(&"rule.exemplar-no-unsafe"), "{ids:?}");
        assert!(!ids.iter().any(|id| id.contains("todo")));
        for proposal in &proposals {
            proposal.rule.validate().expect("valid");
            assert_eq!(proposal.rule.source.kind, RuleSourceKind::Exemplar);
            assert!(!proposal.rule.source.provenance.is_empty());
            assert!(proposal
                .rule
                .source
                .provenance
                .iter()
                .all(|evidence| evidence.digest.is_some()));
        }
    }
}
