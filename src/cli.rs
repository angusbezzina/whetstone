use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::{
    ci_check, detect, detect_patterns, doctor, generate_context, generate_tests, output, resolve,
    rules, status,
};

#[derive(Parser)]
#[command(
    name = "whetstone",
    about = "Whetstone \u{2014} sharpen the tools that write your code.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Detect project dependencies from manifest files
    #[command(alias = "deps")]
    DetectDeps {
        /// Root directory to search for manifest files
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Compare current deps against stored versions in whetstone rules
        #[arg(long)]
        check_drift: bool,

        /// Only output dependencies that have drifted
        #[arg(long)]
        changed_only: bool,

        /// Comma-separated directory patterns to exclude
        #[arg(long)]
        exclude: Option<String>,

        /// Comma-separated directory patterns to include even if normally skipped
        #[arg(long)]
        include: Option<String>,

        /// Compare manifest fingerprints and persist dependency inventory
        #[arg(long)]
        incremental: bool,
    },

    /// Resolve documentation URLs and fetch content for dependencies
    #[command(alias = "resolve")]
    ResolveSources {
        /// JSON input file from detect-deps (default: stdin)
        #[arg(long)]
        input: Option<PathBuf>,

        /// Comma-separated list of dependency names to resolve
        #[arg(long)]
        deps: Option<String>,

        /// Only resolve deps whose content has changed
        #[arg(long)]
        changed_only: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// HTTP request timeout in seconds
        #[arg(long, default_value_t = 15)]
        timeout: u64,

        /// Cache TTL in seconds (default: 7 days)
        #[arg(long, default_value_t = 604800)]
        ttl: u64,

        /// Ignore cache and re-resolve all
        #[arg(long)]
        force_refresh: bool,

        /// Skip deps already resolved in state
        #[arg(long)]
        resume: bool,

        /// Re-resolve only deps in failed state
        #[arg(long)]
        retry_failed: bool,

        /// Number of parallel workers
        #[arg(long)]
        workers: Option<usize>,
    },

    /// Single command from zero to working rules
    Doctor {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Skip pattern detection
        #[arg(long)]
        skip_patterns: bool,

        /// Skip dev dependencies (default)
        #[arg(long, default_value_t = true)]
        skip_dev: bool,

        /// Include dev dependencies
        #[arg(long)]
        include_dev: bool,

        /// Comma-separated dependency names to target
        #[arg(long)]
        deps: Option<String>,

        /// Output only JSON
        #[arg(long)]
        json: bool,

        /// Show full source list in report
        #[arg(long)]
        verbose: bool,

        /// Only resolve changed/stale deps
        #[arg(long)]
        changed_only: bool,

        /// Force re-resolve cached deps
        #[arg(long)]
        refresh: bool,

        /// Resume from last checkpoint
        #[arg(long)]
        resume: bool,

        /// Max deps to resolve this run
        #[arg(long)]
        max_deps: Option<usize>,

        /// Only hand off extraction-ready deps
        #[arg(long)]
        ready_only: bool,

        /// Parallel resolution workers
        #[arg(long)]
        workers: Option<usize>,

        /// Disable fast-first limiting
        #[arg(long)]
        full_run: bool,
    },

    /// Compact project health summary
    Status {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Output only JSON
        #[arg(long)]
        json: bool,

        /// Output only score and label
        #[arg(long)]
        score: bool,

        /// Skip dependency drift check
        #[arg(long)]
        no_drift_check: bool,

        /// Only evaluate rules for drifted deps
        #[arg(long)]
        changed_only: bool,

        /// Show metric trend history
        #[arg(long)]
        history: bool,

        /// Skip recording a metric snapshot
        #[arg(long)]
        no_snapshot: bool,

        /// Output only extraction-ready deps
        #[arg(long)]
        extraction_ready: bool,
    },

    /// Generate agent context files from approved rules
    #[command(alias = "context")]
    GenerateContext {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Comma-separated format names (claude.md, agents.md, .cursorrules, etc.)
        #[arg(long)]
        formats: Option<String>,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,

        /// Output only JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate test files and linter configs from approved rules
    #[command(alias = "tests")]
    GenerateTests {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,

        /// Output only JSON
        #[arg(long)]
        json: bool,
    },

    /// Validate the rule schema and all rule fixtures
    #[command(alias = "validate")]
    ValidateRules {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Mine style patterns from transcripts, git history, and PR comments
    #[command(alias = "patterns")]
    DetectPatterns {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Only analyze data since last execution
        #[arg(long)]
        since_last_run: bool,

        /// Time-bounded analysis (ISO date or relative e.g. "7 days ago")
        #[arg(long)]
        since: Option<String>,

        /// Only output when new patterns are found
        #[arg(long)]
        quiet: bool,

        /// Comma-separated sources (transcript,git,pr)
        #[arg(long, default_value = "transcript,git,pr")]
        sources: String,

        /// Minimum occurrences required to report a pattern
        #[arg(long, default_value_t = 2)]
        min_occurrences: usize,

        /// Scan all agent transcripts, not just project-scoped matches
        #[arg(long)]
        global_transcripts: bool,
    },

    /// Lightweight freshness check for CI/CD
    #[command(alias = "check")]
    CiCheck {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Output JSON only
        #[arg(long)]
        json: bool,

        /// Output as GitHub PR comment markdown
        #[arg(long)]
        pr_comment: bool,

        /// Exit with error on status
        #[arg(long, default_value = "none")]
        fail_on: String,

        /// Skip drift check
        #[arg(long)]
        no_drift_check: bool,

        /// Only check changed deps
        #[arg(long)]
        changed_only: bool,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Commands::DetectDeps {
            project_dir,
            check_drift,
            changed_only,
            exclude,
            include,
            incremental,
        } => {
            let cli_excludes: Vec<String> = exclude
                .map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
                .unwrap_or_default();
            let cli_includes: Vec<String> = include
                .map(|s| s.split(',').map(|i| i.trim().to_string()).collect())
                .unwrap_or_default();

            let do_drift = check_drift || changed_only;
            match detect::detect_deps(
                &project_dir,
                do_drift,
                &cli_excludes,
                &cli_includes,
                incremental,
            ) {
                Ok(mut result) => {
                    if changed_only {
                        if let Some(drift) = result.get("drift") {
                            let changed: Vec<String> = drift
                                .get("changed")
                                .and_then(|c| c.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| {
                                            v.get("name").and_then(|n| n.as_str()).map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            if !changed.is_empty() {
                                if let Some(deps) = result.get_mut("dependencies") {
                                    if let Some(arr) = deps.as_array() {
                                        let filtered: Vec<_> = arr
                                            .iter()
                                            .filter(|d| {
                                                d.get("name")
                                                    .and_then(|n| n.as_str())
                                                    .map(|n| changed.contains(&n.to_string()))
                                                    .unwrap_or(false)
                                            })
                                            .cloned()
                                            .collect();
                                        *deps = serde_json::Value::Array(filtered);
                                    }
                                }
                                result["next_command"] = serde_json::json!(
                                    "Resolve changed sources: whetstone resolve-sources --changed-only"
                                );
                            } else {
                                result["dependencies"] = serde_json::json!([]);
                                result["next_command"] =
                                    serde_json::json!("No changes detected. Rules are current.");
                            }
                        }
                    }
                    output::print_json(&result);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check project directory and manifest files",
                    ));
                    1
                }
            }
        }

        Commands::ResolveSources {
            input,
            deps,
            changed_only,
            project_dir,
            timeout,
            ttl,
            force_refresh,
            resume,
            retry_failed,
            workers,
        } => {
            let deps_data = match load_deps_input(input.as_deref()) {
                Ok(d) => d,
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check input JSON format and network connectivity",
                    ));
                    return 1;
                }
            };

            let filter_deps: Option<Vec<String>> =
                deps.map(|s| s.split(',').map(|d| d.trim().to_string()).collect());

            match resolve::resolve_sources(resolve::ResolveOptions {
                deps_data: &deps_data,
                filter_deps: filter_deps.as_deref(),
                changed_only,
                project_dir: &project_dir,
                timeout,
                ttl,
                force_refresh,
                resume,
                retry_failed,
                workers,
            }) {
                Ok(result) => {
                    output::print_json(&result);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check input JSON format and network connectivity",
                    ));
                    1
                }
            }
        }

        Commands::Doctor {
            project_dir,
            skip_patterns,
            skip_dev: _,
            include_dev,
            deps,
            json,
            verbose,
            changed_only,
            refresh,
            resume,
            max_deps,
            ready_only,
            workers,
            full_run,
        } => {
            let skip_dev = !include_dev;
            let _ = skip_patterns;
            match doctor::doctor(doctor::DoctorOptions {
                project_dir: &project_dir,
                skip_dev,
                json_mode: json,
                deps_filter: deps.as_deref(),
                verbose,
                changed_only,
                refresh,
                resume,
                max_deps,
                ready_only,
                workers,
                full_run,
            }) {
                Ok(result) => {
                    // Remove private fields before output
                    let mut out = result.clone();
                    if let Some(obj) = out.as_object_mut() {
                        let keys: Vec<String> =
                            obj.keys().filter(|k| k.starts_with('_')).cloned().collect();
                        for k in keys {
                            obj.remove(&k);
                        }
                    }
                    output::print_json(&out);
                    if result.get("status").and_then(|s| s.as_str()) == Some("error") {
                        1
                    } else {
                        0
                    }
                }
                Err(e) => {
                    output::print_json(&serde_json::json!({
                        "error": e.to_string(),
                        "recommendations": [],
                        "source_details": [],
                        "next_command": "Check project directory and script dependencies",
                    }));
                    1
                }
            }
        }

        Commands::Status {
            project_dir,
            json,
            score,
            no_drift_check,
            changed_only,
            history,
            no_snapshot,
            extraction_ready,
        } => {
            if extraction_ready {
                let list = status::extraction_ready_list(&project_dir);
                output::print_json(&serde_json::json!(list));
                return 0;
            }

            if history {
                let entries = status::load_metrics_history(&project_dir, 20);
                if json {
                    output::print_json(&serde_json::json!({"history": entries}));
                } else {
                    let report = status::format_history(&entries);
                    eprintln!("{report}");
                    output::print_json(&serde_json::json!({"history": entries}));
                }
                return 0;
            }

            match status::compute_status(&project_dir, !no_drift_check, changed_only) {
                Ok(result) => {
                    if !no_snapshot
                        && result.get("status").and_then(|s| s.as_str()) != Some("not_initialized")
                    {
                        status::snapshot_metrics(&project_dir, &result);
                    }

                    if score {
                        let s = result.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
                        let l = result
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");
                        println!("{s} {l}");
                    } else if json {
                        output::print_json(&result);
                    } else {
                        let report = status::format_human_output(&result);
                        eprintln!("{report}");
                        output::print_json(&result);
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check project directory and whetstone configuration",
                    ));
                    1
                }
            }
        }

        Commands::GenerateContext {
            project_dir,
            formats,
            lang,
            dry_run,
            json,
        } => {
            match generate_context::generate_context(
                &project_dir,
                formats.as_deref(),
                lang.as_deref(),
                dry_run,
            ) {
                Ok(result) => {
                    if !json {
                        let gen = result.get("generated").and_then(|v| v.as_array());
                        if let Some(files) = gen {
                            for f in files {
                                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                                let lines = f.get("lines").and_then(|v| v.as_i64()).unwrap_or(0);
                                let dry = if f
                                    .get("dry_run")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                {
                                    " (dry run)"
                                } else {
                                    ""
                                };
                                eprintln!("  + {path} ({lines} lines){dry}");
                            }
                        }
                    }
                    output::print_json(&result);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check whetstone/rules/ directory for approved rules",
                    ));
                    1
                }
            }
        }

        Commands::GenerateTests {
            project_dir,
            lang,
            dry_run,
            json,
        } => match generate_tests::generate_tests(&project_dir, lang.as_deref(), dry_run) {
            Ok(result) => {
                if !json {
                    if let Some(gen) = result.get("generated") {
                        if let Some(tests) = gen.get("tests").and_then(|v| v.as_array()) {
                            for f in tests {
                                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                                eprintln!("  + {path}");
                            }
                        }
                        if let Some(lints) = gen.get("lint_configs").and_then(|v| v.as_array()) {
                            for f in lints {
                                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                                eprintln!("  + {path}");
                            }
                        }
                    }
                }
                output::print_json(&result);
                0
            }
            Err(e) => {
                output::print_json(&output::error_json(
                    &e.to_string(),
                    "Check whetstone/rules/ directory for approved rules",
                ));
                1
            }
        },

        Commands::ValidateRules { project_dir } => {
            let (report, ok) = rules::validate_schema_and_fixtures(&project_dir);
            print!("{report}");
            if ok {
                0
            } else {
                1
            }
        }

        Commands::DetectPatterns {
            project_dir,
            since_last_run,
            since,
            quiet,
            sources,
            min_occurrences,
            global_transcripts,
        } => {
            let source_set = detect_patterns::parse_sources(&sources);
            if source_set.is_empty() {
                output::print_json(&output::error_json(
                    "No valid sources specified. Use: transcript, git, pr",
                    "Pass --sources with at least one of transcript, git, pr",
                ));
                return 1;
            }
            match detect_patterns::detect_patterns(detect_patterns::DetectPatternsOptions {
                project_dir: &project_dir,
                sources: source_set,
                since,
                since_last_run,
                quiet,
                min_occurrences,
                global_transcripts,
            }) {
                Ok(result) => {
                    output::print_json(&result);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check project directory and source availability",
                    ));
                    1
                }
            }
        }

        Commands::CiCheck {
            project_dir,
            json,
            pr_comment,
            fail_on,
            no_drift_check,
            changed_only,
        } => match ci_check::ci_check(&project_dir, !no_drift_check, changed_only) {
            Ok(result) => {
                if pr_comment {
                    println!("{}", ci_check::format_pr_comment(&result));
                } else if json {
                    output::print_json(&result);
                } else {
                    let s = result
                        .get("freshness_status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let label = result
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    let score = result.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
                    eprintln!(
                        "Whetstone: [{}] {} (score: {}/100)",
                        s.to_uppercase(),
                        label,
                        score
                    );
                    output::print_json(&result);
                }

                let freshness = result
                    .get("freshness_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match fail_on.as_str() {
                    "stale" if freshness == "stale" => 1,
                    "needs_review" if freshness == "stale" || freshness == "needs_review" => 1,
                    _ => 0,
                }
            }
            Err(e) => {
                output::print_json(&output::error_json(
                    &e.to_string(),
                    "Check project directory and script dependencies",
                ));
                1
            }
        },
    }
}

fn load_deps_input(input: Option<&std::path::Path>) -> anyhow::Result<serde_json::Value> {
    if let Some(path) = input {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    } else {
        Ok(serde_json::from_reader(std::io::stdin())?)
    }
}
