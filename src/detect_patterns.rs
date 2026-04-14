//! Pattern mining from agent transcripts, git history, and GitHub PR comments.
//!
//! This is the native Rust port of the legacy `scripts/detect-patterns.py`
//! helper. The JSON contract is preserved so downstream consumers (doctor,
//! skill workflow, tests) can swap to the Rust implementation without caring
//! which backend produced the output.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde_json::{json, Value};
use walkdir::WalkDir;

/// Known agent transcript directories relative to the user's home directory.
/// Each agent stores conversation history as JSONL under its own subtree.
const TRANSCRIPT_DIRS: &[&str] = &[
    ".claude/projects",
    ".cursor/projects",
    ".cline/projects",
    ".continue/sessions",
    ".codex/sessions",
    ".goose/sessions",
    ".roo/projects",
    ".agents/sessions",
    ".config/opencode/sessions",
    ".windsurf/sessions",
];

const STYLE_CONFIG_FILES: &[&str] = &[
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.json",
    ".eslintrc.yml",
    "ruff.toml",
    "pyproject.toml",
    "rustfmt.toml",
    "biome.json",
    "biome.jsonc",
    ".prettierrc",
    ".prettierrc.json",
    ".editorconfig",
    "deno.json",
    "deno.jsonc",
];

fn directive_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)\b(always\s+use)\b",
            r"(?i)\b(never\s+use)\b",
            r"(?i)\b(prefer\s+\w+\s+over)\b",
            r"(?i)\b(don'?t\s+use)\b",
            r"(?i)\b(make\s+sure\s+you)\b",
            r"(?i)\b(never\s+do)\b",
            r"(?i)\b(always\s+prefer)\b",
            r"(?i)\b(should\s+always)\b",
            r"(?i)\b(must\s+use)\b",
            r"(?i)\b(do\s+not\s+use)\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static directive regex"))
        .collect()
    })
}

fn correction_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)that'?s\s+not\s+how\s+we",
            r"(?i)we\s+use\s+\w+\s+here",
            r"(?i)change\s+this\s+to",
            r"(?i)should\s+be\s+\w+\s+instead",
            r"(?i)let'?s\s+use\s+\w+\s+instead",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static correction regex"))
        .collect()
    })
}

fn version_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)use\s+the\s+new",
            r"(?i)v\d+\s+way",
            r"(?i)latest\s+api",
            r"(?i)\bdeprecated\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static version regex"))
        .collect()
    })
}

fn style_keywords() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(format|style|convention|naming|pattern|approach|standard|consistent|refactor|rename|rewrite|restructure)\b",
        )
        .expect("static style keyword regex")
    })
}

fn git_style_patterns() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(fix\s*style|format|lint|convention|refactor:\s*rename|refactor:\s*style|code\s*style|formatting|clean\s*up|standardize)\b",
        )
        .expect("static git style regex")
    })
}

fn key_extraction_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)(always\s+use\s+[\w\s]+)",
            r"(?i)(never\s+use\s+[\w\s]+)",
            r"(?i)(prefer\s+\w+\s+over\s+\w+)",
            r"(?i)(don'?t\s+use\s+[\w\s]+)",
            r"(?i)(use\s+\w+\s+instead\s+of\s+\w+)",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static key extraction regex"))
        .collect()
    })
}

fn relative_since_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(\d+)\s+(day|week|month)s?\s+ago").expect("static relative since regex")
    })
}

/// Public options for the Rust `detect-patterns` subcommand.
pub struct DetectPatternsOptions<'a> {
    pub project_dir: &'a Path,
    pub sources: BTreeSet<String>,
    pub since: Option<String>,
    pub since_last_run: bool,
    pub quiet: bool,
    pub min_occurrences: usize,
    pub global_transcripts: bool,
}

/// Entry point used by the CLI layer.
pub fn detect_patterns(opts: DetectPatternsOptions) -> Result<Value> {
    // Resolve --since-last-run against persisted timestamp.
    let last_run_file = opts.project_dir.join("whetstone").join(".last-run");
    let mut effective_since = opts.since.clone();
    if opts.since_last_run && last_run_file.exists() {
        if let Ok(ts) = fs::read_to_string(&last_run_file) {
            let trimmed = ts.trim().to_string();
            if !trimmed.is_empty() {
                effective_since = Some(trimmed);
            }
        }
    }

    let since_dt = effective_since.as_deref().and_then(parse_since_to_datetime);

    let mut all_patterns: Vec<Pattern> = Vec::new();
    let mut sources_analyzed: BTreeMap<String, Value> = BTreeMap::new();

    if opts.sources.contains("transcript") {
        if opts.global_transcripts {
            eprintln!(
                "WARNING: --global-transcripts scans ALL agent transcripts across your home directory. This may include conversations from other projects. Use with care."
            );
        }
        let (pats, stats) = mine_transcripts(opts.project_dir, since_dt, opts.global_transcripts);
        all_patterns.extend(pats);
        sources_analyzed.insert("transcript".to_string(), stats);
    }

    if opts.sources.contains("git") {
        let (pats, stats) = mine_git_history(opts.project_dir, effective_since.as_deref());
        all_patterns.extend(pats);
        sources_analyzed.insert("git".to_string(), stats);
    }

    if opts.sources.contains("pr") {
        let (pats, stats) = mine_pr_comments(opts.project_dir, effective_since.as_deref());
        all_patterns.extend(pats);
        sources_analyzed.insert("pr".to_string(), stats);
    }

    let mut filtered = apply_strictness_filters(all_patterns, opts.min_occurrences);
    add_suggested_rules(&mut filtered);

    if opts.quiet && filtered.is_empty() {
        return Ok(json!({
            "patterns": [],
            "sources_analyzed": sources_analyzed,
            "next_command": "No patterns found. Proceed to extraction.",
        }));
    }

    // Update .last-run timestamp.
    let whetstone_dir = opts.project_dir.join("whetstone");
    let _ = fs::create_dir_all(&whetstone_dir);
    let _ = fs::write(&last_run_file, Utc::now().to_rfc3339());

    let next_command = if filtered.is_empty() {
        "No patterns found. Proceed to extraction."
    } else {
        "Review patterns and approve as rules during extraction"
    };

    let patterns_json: Vec<Value> = filtered.iter().map(Pattern::to_json).collect();

    Ok(json!({
        "patterns": patterns_json,
        "sources_analyzed": sources_analyzed,
        "next_command": next_command,
    }))
}

/// Internal representation of a mined pattern.
#[derive(Debug, Clone)]
struct Pattern {
    description: String,
    source: &'static str,
    occurrences: usize,
    confidence: &'static str,
    sessions: Vec<String>,
    example_quotes: Vec<String>,
    last_seen: DateTime<Utc>,
    score: f64,
    suggested_rule: Option<Value>,
}

impl Pattern {
    fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("description".into(), json!(self.description));
        obj.insert("source".into(), json!(self.source));
        obj.insert("occurrences".into(), json!(self.occurrences));
        obj.insert("confidence".into(), json!(self.confidence));
        obj.insert("sessions".into(), json!(self.sessions));
        obj.insert("example_quotes".into(), json!(self.example_quotes));
        obj.insert("last_seen".into(), json!(self.last_seen.to_rfc3339()));
        obj.insert("score".into(), json!(self.score));
        if let Some(rule) = &self.suggested_rule {
            obj.insert("suggested_rule".into(), rule.clone());
        }
        Value::Object(obj)
    }
}

// ----- Source 1: Conversation Transcripts -----

fn project_transcript_matches(project_dir: &Path, transcript_path: &Path) -> bool {
    let project_name = match project_dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
    {
        Some(n) => n,
        None => return false,
    };
    transcript_path
        .to_string_lossy()
        .to_lowercase()
        .contains(&project_name)
}

fn mine_transcripts(
    project_dir: &Path,
    since: Option<DateTime<Utc>>,
    global_transcripts: bool,
) -> (Vec<Pattern>, Value) {
    let mut patterns: Vec<Pattern> = Vec::new();
    let mut files_count: usize = 0;
    let mut messages_count: usize = 0;

    let home = match home_dir() {
        Some(h) => h,
        None => {
            return (
                patterns,
                json!({
                    "files": 0,
                    "messages": 0,
                    "scoped": !global_transcripts,
                }),
            );
        }
    };

    let mut jsonl_files: Vec<PathBuf> = Vec::new();
    for rel_dir in TRANSCRIPT_DIRS {
        let transcript_dir = home.join(rel_dir);
        if !transcript_dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&transcript_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.into_path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if global_transcripts || project_transcript_matches(project_dir, &path) {
                jsonl_files.push(path);
            }
        }
    }

    files_count = jsonl_files.len().max(files_count);

    if jsonl_files.is_empty() {
        return (
            patterns,
            json!({
                "files": files_count,
                "messages": messages_count,
                "scoped": !global_transcripts,
            }),
        );
    }

    // Group raw style signals by extracted key phrase.
    let mut signal_groups: HashMap<String, Vec<SignalHit>> = HashMap::new();

    for jsonl_file in &jsonl_files {
        let session_id = jsonl_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());

        let file = match File::open(jsonl_file) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // UnicodeDecodeError equivalent — stop reading this file
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only look at user/human messages.
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .or_else(|| msg.get("type").and_then(|v| v.as_str()))
                .unwrap_or("");
            if role != "user" && role != "human" {
                continue;
            }

            let content = extract_message_content(&msg);
            if content.trim().is_empty() {
                continue;
            }

            messages_count += 1;

            // Timestamp filter.
            if let Some(since_dt) = since {
                let ts = msg
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .or_else(|| msg.get("created_at").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if !ts.is_empty() {
                    if let Some(msg_time) = parse_iso8601(ts) {
                        if msg_time < since_dt {
                            continue;
                        }
                    }
                }
            }

            // Style signal detection.
            let mut matched = directive_patterns().iter().any(|p| p.is_match(&content));
            if !matched {
                matched = correction_patterns().iter().any(|p| p.is_match(&content));
            }
            if !matched && style_keywords().is_match(&content) {
                // Only count keyword matches if they also hit a version pattern.
                if version_patterns().iter().any(|p| p.is_match(&content)) {
                    matched = true;
                }
            }

            if matched {
                let desc: String = content
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .trim()
                    .to_string();
                let key = extract_pattern_key(&content);
                signal_groups.entry(key).or_default().push(SignalHit {
                    text: desc,
                    session: session_id.clone(),
                });
            }
        }
    }

    // Convert groups into Patterns.
    // Iterate in stable order to make tests deterministic.
    let mut keys: Vec<&String> = signal_groups.keys().collect();
    keys.sort();
    for key in keys {
        let signals = &signal_groups[key];
        let sessions: Vec<String> = {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for s in signals {
                seen.insert(s.session.clone());
            }
            seen.into_iter().take(10).collect()
        };
        let confidence = if sessions.len() >= 3 {
            "high"
        } else {
            "medium"
        };
        patterns.push(Pattern {
            description: key.clone(),
            source: "transcript",
            occurrences: signals.len(),
            confidence,
            sessions,
            example_quotes: signals.iter().take(3).map(|s| s.text.clone()).collect(),
            last_seen: Utc::now(),
            score: 0.0,
            suggested_rule: None,
        });
    }

    (
        patterns,
        json!({
            "files": jsonl_files.len(),
            "messages": messages_count,
            "scoped": !global_transcripts,
        }),
    )
}

struct SignalHit {
    text: String,
    session: String,
}

fn extract_message_content(msg: &Value) -> String {
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    let mut out = String::new();
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        for block in arr {
            if let Some(obj) = block.as_object() {
                if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                        out.push_str(t);
                        out.push(' ');
                    }
                }
            } else if let Some(s) = block.as_str() {
                out.push_str(s);
                out.push(' ');
            }
        }
    }
    out
}

fn extract_pattern_key(text: &str) -> String {
    let trimmed = text.trim();
    for pat in key_extraction_patterns() {
        if let Some(m) = pat.captures(trimmed).and_then(|c| c.get(1)) {
            let raw = m.as_str().trim();
            let truncated: String = raw.chars().take(100).collect();
            return truncated;
        }
    }
    trimmed
        .chars()
        .take(80)
        .collect::<String>()
        .trim()
        .to_string()
}

// ----- Source 2: Git History -----

fn mine_git_history(project_dir: &Path, since: Option<&str>) -> (Vec<Pattern>, Value) {
    let mut patterns: Vec<Pattern> = Vec::new();
    let mut commit_count: usize = 0;

    if !project_dir.join(".git").exists() {
        return (patterns, json!({ "commits": 0 }));
    }

    let mut args: Vec<String> = vec![
        "log".into(),
        "--oneline".into(),
        "--no-merges".into(),
        "-500".into(),
    ];
    if let Some(s) = since {
        args.push("--since".into());
        args.push(s.to_string());
    }

    let output = run_command("git", &args, Some(project_dir));
    let Some(log) = output else {
        return (patterns, json!({ "commits": 0 }));
    };

    let mut style_commits: Vec<(String, String)> = Vec::new();
    for line in log.trim().split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        commit_count += 1;
        let (sha, message) = match line.split_once(' ') {
            Some(pair) => pair,
            None => continue,
        };
        if git_style_patterns().is_match(message) {
            style_commits.push((sha.to_string(), message.to_string()));
        }
    }

    let mut commit_groups: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for (_, message) in &style_commits {
        let msg_lower = message.to_lowercase();
        let bucket: &'static str =
            if msg_lower.contains("format") || msg_lower.contains("formatting") {
                "Code formatting standardization"
            } else if msg_lower.contains("lint") {
                "Linting fixes"
            } else if msg_lower.contains("rename") {
                "Naming convention changes"
            } else if msg_lower.contains("style") {
                "Style fixes"
            } else if msg_lower.contains("refactor") {
                "Refactoring patterns"
            } else {
                "Other style changes"
            };
        commit_groups
            .entry(bucket)
            .or_default()
            .push(message.clone());
    }

    for (desc, commits) in commit_groups {
        if commits.len() >= 2 {
            let confidence = if commits.len() >= 5 { "high" } else { "medium" };
            patterns.push(Pattern {
                description: desc.to_string(),
                source: "git",
                occurrences: commits.len(),
                confidence,
                sessions: Vec::new(),
                example_quotes: commits.iter().take(3).cloned().collect(),
                last_seen: Utc::now(),
                score: 0.0,
                suggested_rule: None,
            });
        }
    }

    // Config file changes.
    for config_file in STYLE_CONFIG_FILES {
        let mut cfg_args: Vec<String> = vec!["log".into(), "--oneline".into(), "-10".into()];
        if let Some(s) = since {
            cfg_args.push(format!("--since={s}"));
        }
        cfg_args.push("--".into());
        cfg_args.push((*config_file).to_string());

        let Some(cfg_out) = run_command("git", &cfg_args, Some(project_dir)) else {
            continue;
        };
        if cfg_out.trim().is_empty() {
            continue;
        }
        let changes: Vec<String> = cfg_out
            .trim()
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        if changes.len() >= 2 {
            patterns.push(Pattern {
                description: format!(
                    "Frequent changes to {config_file} (style preference evolution)"
                ),
                source: "git",
                occurrences: changes.len(),
                confidence: "medium",
                sessions: Vec::new(),
                example_quotes: changes.iter().take(3).cloned().collect(),
                last_seen: Utc::now(),
                score: 0.0,
                suggested_rule: None,
            });
        }
    }

    (patterns, json!({ "commits": commit_count }))
}

// ----- Source 3: GitHub PR Comments -----

fn resolve_gh_repo(project_dir: &Path) -> Option<String> {
    run_command(
        "gh",
        &[
            "repo".into(),
            "view".into(),
            "--json".into(),
            "nameWithOwner".into(),
            "--jq".into(),
            ".nameWithOwner".into(),
        ],
        Some(project_dir),
    )
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

fn mine_pr_comments(project_dir: &Path, _since: Option<&str>) -> (Vec<Pattern>, Value) {
    let mut patterns: Vec<Pattern> = Vec::new();
    let mut comment_count: usize = 0;

    if !has_command("gh") {
        return (patterns, json!({ "comments": 0 }));
    }

    let Some(repo_slug) = resolve_gh_repo(project_dir) else {
        return (patterns, json!({ "comments": 0 }));
    };

    let Some(list_json) = run_command(
        "gh",
        &[
            "pr".into(),
            "list".into(),
            "--state".into(),
            "closed".into(),
            "--limit".into(),
            "20".into(),
            "--json".into(),
            "number,title,closedAt".into(),
        ],
        Some(project_dir),
    ) else {
        return (patterns, json!({ "comments": 0 }));
    };

    let prs: Value = match serde_json::from_str(&list_json) {
        Ok(v) => v,
        Err(_) => return (patterns, json!({ "comments": 0 })),
    };

    let mut style_comments: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(arr) = prs.as_array() {
        for pr in arr {
            let pr_num = match pr.get("number").and_then(|v| v.as_i64()) {
                Some(n) => n,
                None => continue,
            };
            let Some(body) = run_command(
                "gh",
                &[
                    "api".into(),
                    format!("repos/{repo_slug}/pulls/{pr_num}/comments"),
                    "--jq".into(),
                    ".[].body".into(),
                ],
                Some(project_dir),
            ) else {
                continue;
            };
            for comment in body.trim().split('\n') {
                if comment.trim().is_empty() {
                    continue;
                }
                comment_count += 1;

                let directive_hit = directive_patterns().iter().any(|p| p.is_match(comment));
                let correction_hit = correction_patterns().iter().any(|p| p.is_match(comment));
                if style_keywords().is_match(comment) || directive_hit || correction_hit {
                    let key = extract_pattern_key(comment);
                    let clipped: String = comment.chars().take(200).collect();
                    style_comments.entry(key).or_default().push(clipped);
                }
            }
        }
    }

    let mut keys: Vec<&String> = style_comments.keys().collect();
    keys.sort();
    for key in keys {
        let comments = &style_comments[key];
        if comments.len() >= 2 {
            let confidence = if comments.len() >= 3 {
                "high"
            } else {
                "medium"
            };
            patterns.push(Pattern {
                description: key.clone(),
                source: "pr",
                occurrences: comments.len(),
                confidence,
                sessions: Vec::new(),
                example_quotes: comments.iter().take(3).cloned().collect(),
                last_seen: Utc::now(),
                score: 0.0,
                suggested_rule: None,
            });
        }
    }

    (patterns, json!({ "comments": comment_count }))
}

// ----- Pattern processing -----

fn apply_strictness_filters(patterns: Vec<Pattern>, min_occurrences: usize) -> Vec<Pattern> {
    let now = Utc::now();
    let mut filtered: Vec<Pattern> = patterns
        .into_iter()
        .filter(|p| p.occurrences >= min_occurrences)
        .map(|mut p| {
            let days_ago = (now - p.last_seen).num_days();
            let base = p.occurrences as f64;
            p.score = if days_ago <= 30 {
                base * 3.0
            } else if days_ago <= 90 {
                base * 1.5
            } else {
                base
            };
            p
        })
        .collect();

    filtered.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut deduped: Vec<Pattern> = Vec::new();
    for p in filtered {
        let key: String = p
            .description
            .to_lowercase()
            .chars()
            .take(50)
            .collect::<String>();
        if seen_keys.insert(key) {
            deduped.push(p);
        }
    }
    deduped
}

fn add_suggested_rules(patterns: &mut [Pattern]) {
    for p in patterns.iter_mut() {
        let desc = p.description.clone();
        p.suggested_rule = Some(json!({
            "description": format!("Code SHOULD follow the convention: {desc}"),
            "severity": "should",
            "category": "convention",
            "signals": [
                {
                    "strategy": "pattern",
                    "description": format!("Check for adherence to: {desc}"),
                }
            ],
        }));
    }
}

// ----- Helpers -----

fn run_command(program: &str, args: &[String], cwd: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn has_command(name: &str) -> bool {
    run_command("which", &[name.to_string()], None).is_some()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_iso8601(raw: &str) -> Option<DateTime<Utc>> {
    let normalized = raw.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_since_to_datetime(raw: &str) -> Option<DateTime<Utc>> {
    if let Some(dt) = parse_iso8601(raw) {
        return Some(dt);
    }
    let caps = relative_since_regex().captures(raw)?;
    let n: i64 = caps.get(1)?.as_str().parse().ok()?;
    let unit = caps.get(2)?.as_str().to_lowercase();
    let now = Utc::now();
    match unit.as_str() {
        "day" => Some(now - Duration::days(n)),
        "week" => Some(now - Duration::weeks(n)),
        "month" => Some(now - Duration::days(n * 30)),
        _ => None,
    }
}

/// Parse a comma-separated sources string into the validated set.
pub fn parse_sources(raw: &str) -> BTreeSet<String> {
    let valid: BTreeSet<&str> = ["transcript", "git", "pr"].iter().copied().collect();
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| valid.contains(s.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sources_filters_invalid() {
        let s = parse_sources("transcript,git,foobar, pr");
        assert!(s.contains("transcript"));
        assert!(s.contains("git"));
        assert!(s.contains("pr"));
        assert!(!s.contains("foobar"));
    }

    #[test]
    fn extract_pattern_key_matches_directive() {
        let key = extract_pattern_key("Please always use snake_case here.");
        assert!(key.to_lowercase().starts_with("always use"));
    }

    #[test]
    fn extract_pattern_key_falls_back_to_prefix() {
        let key = extract_pattern_key("Some long style message without directive phrasing at all.");
        assert!(key.len() <= 80);
    }

    #[test]
    fn parse_since_handles_relative() {
        let dt = parse_since_to_datetime("7 days ago").expect("relative since");
        let now = Utc::now();
        assert!((now - dt).num_days() >= 6);
    }

    #[test]
    fn parse_since_handles_iso() {
        let dt = parse_since_to_datetime("2024-01-01T00:00:00Z").expect("iso since");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2024-01-01");
    }

    #[test]
    fn strictness_floor_filters_low_occurrence() {
        let patterns = vec![Pattern {
            description: "always use spaces".into(),
            source: "transcript",
            occurrences: 1,
            confidence: "medium",
            sessions: vec![],
            example_quotes: vec![],
            last_seen: Utc::now(),
            score: 0.0,
            suggested_rule: None,
        }];
        let filtered = apply_strictness_filters(patterns, 2);
        assert!(filtered.is_empty());
    }
}
