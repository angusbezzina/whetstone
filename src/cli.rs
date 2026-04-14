use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use crate::{
    bench, check, ci_check, detect, detect_patterns, doctor, eval, generate_context,
    generate_tests, layers, output, personal, resolve, review, rules, status, triggers, update,
};

#[derive(Parser)]
#[command(
    name = "whetstone",
    about = "Whetstone \u{2014} sharpen the tools that write your code.",
    version
)]
struct Cli {
    /// Output machine-readable JSON instead of human-friendly text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum ReviewAction {
    /// Show full context for a single rule
    Show { rule_id: String },
    /// Build a review queue from extraction-handoff + refresh-diff artifacts
    Queue,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan project and detect dependencies (or run --personal / --hooks / --ci setup)
    #[command(name = "init", visible_alias = "deps", alias = "detect-deps")]
    Init {
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

        /// Scaffold whetstone/.personal/ (rules, evals, lint, context) + .gitignore entries
        #[arg(long)]
        personal: bool,

        /// Install session + post-merge git hooks under .githooks/
        #[arg(long)]
        hooks: bool,

        /// Generate .github/workflows/whetstone-check.yml for scheduled freshness checks
        #[arg(long)]
        ci: bool,

        /// Schedule for the CI workflow (daily|weekly|biweekly|monthly or a 5-field cron)
        #[arg(long, default_value = "weekly")]
        schedule: String,
    },

    /// Resolve documentation URLs and fetch content for dependencies
    #[command(
        name = "set-sources",
        visible_alias = "sources",
        alias = "resolve-sources"
    )]
    SetSources {
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

    /// Bootstrap from zero to working rules
    #[command(visible_alias = "start")]
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

    /// Project health summary and drift detection
    Status {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

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
    #[command(name = "context", alias = "generate-context")]
    Context {
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

        /// Render personal-layer-only context into whetstone/.personal/context/
        #[arg(long)]
        personal: bool,
    },

    /// Generate test files and linter configs from approved rules
    #[command(name = "tests", alias = "generate-tests")]
    Tests {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,

        /// Emit personal-layer tests into whetstone/.personal/evals/
        #[arg(long)]
        personal: bool,
    },

    /// Move a rule between layers (personal → project → team)
    Promote {
        /// Rule id (e.g., "reqwest.set-timeout") to move
        rule_id: String,

        /// Target layer (personal|project|team)
        #[arg(long = "to", default_value = "project")]
        to: String,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Keep the source rule in place (copy instead of move)
        #[arg(long)]
        keep_source: bool,
    },

    /// Show the 4-layer rule merge summary (personal + project + team + built-in)
    Layers {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,
    },

    /// Validate the rule schema and all rule fixtures
    #[command(name = "validate", alias = "validate-rules")]
    Validate {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Mine style patterns from transcripts, git history, and PR comments
    #[command(name = "patterns", alias = "detect-patterns")]
    Patterns {
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

    /// Scan source files for rule violations using tree-sitter and regex signals
    Check {
        /// Paths to scan (defaults to the project directory)
        paths: Vec<PathBuf>,

        /// Project root directory (used to locate whetstone/rules/)
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Only run the named rule (may be repeated; comma-separated accepted)
        #[arg(long)]
        rule: Option<String>,

        /// Treat violations as exit-zero (for preview runs)
        #[arg(long)]
        no_fail: bool,
    },

    /// Lightweight freshness check for CI/CD
    #[command(name = "ci", alias = "ci-check")]
    Ci {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

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

    /// Check for dependency drift and re-resolve changed sources
    #[command(alias = "refresh-rules")]
    Refresh {
        /// Project directory
        #[arg(long, default_value = ".")]
        project_dir: String,

        /// Exit non-zero if drift exists (for CI)
        #[arg(long)]
        check: bool,
    },

    /// AI evaluation: threshold gating, eval requests, calibration
    Eval {
        /// Action: generate, run, or calibrate
        action: String,

        /// Project directory
        #[arg(long, default_value = ".")]
        project_dir: String,

        /// Collect verdicts from agent (for run --collect and calibrate --collect)
        #[arg(long)]
        collect: bool,

        /// Only run deterministic checks, skip AI requests
        #[arg(long)]
        deterministic_only: bool,

        /// Filter by language
        #[arg(long)]
        lang: Option<String>,

        /// Preview without writing files
        #[arg(long)]
        dry_run: bool,
    },

    /// Review rules by lifecycle status (candidate / approved / denied / deprecated)
    Review {
        #[command(subcommand)]
        action: Option<ReviewAction>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by status when no subcommand is supplied
        #[arg(long)]
        status: Option<String>,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,
    },

    /// Apply a lifecycle transition to a rule (approve / deny / deprecate / supersede)
    Apply {
        /// Rule id to transition
        rule_id: Option<String>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Approve the rule (candidate → approved)
        #[arg(long, conflicts_with_all = ["deny", "deprecate", "supersede", "batch"])]
        approve: bool,

        /// Deny the rule (candidate → denied). Requires --reason.
        #[arg(long, conflicts_with_all = ["approve", "deprecate", "supersede", "batch"])]
        deny: bool,

        /// Deprecate the rule (approved → deprecated). Requires --reason.
        #[arg(long, conflicts_with_all = ["approve", "deny", "supersede", "batch"])]
        deprecate: bool,

        /// Supersede: deprecate and record superseded_by. Requires --superseded-by.
        #[arg(long, conflicts_with_all = ["approve", "deny", "deprecate", "batch"])]
        supersede: bool,

        /// Reason for denial or deprecation (required for --deny / --deprecate / --supersede)
        #[arg(long)]
        reason: Option<String>,

        /// Replacement rule id (required for --supersede)
        #[arg(long = "superseded-by")]
        superseded_by: Option<String>,

        /// Record this actor in the audit log
        #[arg(long)]
        actor: Option<String>,

        /// Batch file (JSON array of {rule_id, action, reason?, superseded_by?})
        #[arg(long)]
        batch: Option<PathBuf>,

        /// Preview the transition without writing files
        #[arg(long)]
        dry_run: bool,
    },

    /// Run rule-quality benchmark corpus and report precision/recall/F1
    Bench {
        /// Action: run | snapshot
        #[arg(default_value = "run")]
        action: String,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Corpus directory (defaults to <project_dir>/benchmarks)
        #[arg(long)]
        corpus_dir: Option<PathBuf>,

        /// Only run scenarios whose name contains this substring
        #[arg(long)]
        scenario: Option<String>,

        /// Minimum F1 score per scenario before failing (0..=1)
        #[arg(long, default_value_t = 1.0)]
        min_f1: f64,

        /// Exit non-zero if any scenario regresses below --min-f1
        #[arg(long)]
        check: bool,
    },

    /// Update whetstone to the latest release
    Update {
        /// Only check for updates, don't install
        #[arg(long)]
        check: bool,

        /// Force update even if already on the latest version
        #[arg(long)]
        force: bool,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let json_mode = cli.json || output::is_piped();

    match cli.command {
        Commands::Init {
            project_dir,
            check_drift,
            changed_only,
            exclude,
            include,
            incremental,
            personal,
            hooks,
            ci,
            schedule,
        } => {
            // Setup flags short-circuit detection. They can compose — e.g.
            // `wh init --personal --hooks --ci --schedule=weekly`.
            if personal || hooks || ci {
                let mut setup = serde_json::Map::new();
                setup.insert("status".to_string(), serde_json::json!("ok"));

                if personal {
                    match personal::init_personal(&project_dir) {
                        Ok(v) => {
                            setup.insert("personal".to_string(), v);
                        }
                        Err(e) => {
                            output::print_json(&output::error_json(
                                &e.to_string(),
                                "Check filesystem permissions on the project directory",
                            ));
                            return 1;
                        }
                    }
                }
                if hooks {
                    match triggers::install_hooks(&project_dir, &triggers::HookOptions::all()) {
                        Ok(v) => {
                            setup.insert("hooks".to_string(), v);
                        }
                        Err(e) => {
                            output::print_json(&output::error_json(
                                &e.to_string(),
                                "Ensure the project is a git repository before running --hooks",
                            ));
                            return 1;
                        }
                    }
                }
                if ci {
                    match triggers::install_ci_workflow(&project_dir, &schedule) {
                        Ok(v) => {
                            setup.insert("ci".to_string(), v);
                        }
                        Err(e) => {
                            output::print_json(&output::error_json(
                                &e.to_string(),
                                "Pass --schedule=daily|weekly|biweekly|monthly or a 5-field cron expression",
                            ));
                            return 1;
                        }
                    }
                }

                output::print_json(&serde_json::Value::Object(setup));
                return 0;
            }

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
                                    "Resolve changed sources: wh set-sources --changed-only"
                                );
                            } else {
                                result["dependencies"] = serde_json::json!([]);
                                result["next_command"] =
                                    serde_json::json!("No changes detected. Rules are current.");
                            }
                        }
                    }
                    if json_mode {
                        output::print_json(&result);
                    } else {
                        println!("{}", detect::format_human_output(&result));
                    }
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

        Commands::SetSources {
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
                    if json_mode {
                        output::print_json(&result);
                    } else {
                        println!("{}", resolve::format_human_output(&result));
                    }
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
                json_mode,
                deps_filter: deps.as_deref(),
                verbose,
                changed_only,
                refresh,
                resume,
                max_deps,
                ready_only,
                workers,
                full_run,
                trigger: "doctor",
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
                    if json_mode {
                        output::print_json(&out);
                    }
                    // Doctor prints its own human report internally via format_report
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
                if json_mode {
                    output::print_json(&serde_json::json!({"history": entries}));
                } else {
                    let report = status::format_history(&entries);
                    println!("{report}");
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
                    } else if json_mode {
                        output::print_json(&result);
                    } else {
                        let report = status::format_human_output(&result);
                        println!("{report}");
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

        Commands::Context {
            project_dir,
            formats,
            lang,
            dry_run,
            personal,
        } => {
            match generate_context::generate_context(
                &project_dir,
                formats.as_deref(),
                lang.as_deref(),
                dry_run,
                personal,
            ) {
                Ok(result) => {
                    if json_mode {
                        output::print_json(&result);
                    } else {
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
                                println!("  + {path} ({lines} lines){dry}");
                            }
                        }
                    }
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

        Commands::Tests {
            project_dir,
            lang,
            dry_run,
            personal,
        } => match generate_tests::generate_tests(&project_dir, lang.as_deref(), dry_run, personal)
        {
            Ok(result) => {
                if json_mode {
                    output::print_json(&result);
                } else {
                    if let Some(gen) = result.get("generated") {
                        if let Some(tests) = gen.get("tests").and_then(|v| v.as_array()) {
                            for f in tests {
                                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                                println!("  + {path}");
                            }
                        }
                        if let Some(lints) = gen.get("lint_configs").and_then(|v| v.as_array()) {
                            for f in lints {
                                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                                println!("  + {path}");
                            }
                        }
                    }
                }
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

        Commands::Validate { project_dir } => {
            let (report, ok) = rules::validate_schema_and_fixtures(&project_dir);
            print!("{report}");
            if ok {
                0
            } else {
                1
            }
        }

        Commands::Patterns {
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
                    if json_mode {
                        output::print_json(&result);
                    } else {
                        let count = result
                            .get("patterns")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        if count == 0 {
                            println!("No patterns found.");
                        } else {
                            println!("Found {count} pattern(s):");
                            if let Some(patterns) =
                                result.get("patterns").and_then(|v| v.as_array())
                            {
                                for p in patterns {
                                    let desc = p
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("?");
                                    let src =
                                        p.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                                    let occ =
                                        p.get("occurrences").and_then(|v| v.as_u64()).unwrap_or(0);
                                    println!("  [{src}] {desc} ({occ} occurrences)");
                                }
                            }
                        }
                    }
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

        Commands::Check {
            paths,
            project_dir,
            lang,
            rule,
            no_fail,
        } => {
            let scan_paths: Vec<PathBuf> = if paths.is_empty() {
                vec![project_dir.clone()]
            } else {
                paths
            };
            let rule_filter: Option<Vec<String>> =
                rule.map(|s| s.split(',').map(|r| r.trim().to_string()).collect());
            match check::run(check::CheckOptions {
                project_dir: &project_dir,
                scan_paths: &scan_paths,
                lang_filter: lang.as_deref(),
                rule_filter: rule_filter.as_deref(),
            }) {
                Ok(result) => {
                    let violations_count = result
                        .get("violations_count")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let config_issues_count = result
                        .get("config_issues_count")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if json_mode {
                        output::print_json(&result);
                    } else {
                        println!("{}", check::format_human_output(&result));
                    }
                    if (violations_count > 0 || config_issues_count > 0) && !no_fail {
                        1
                    } else {
                        0
                    }
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check project directory and whetstone/rules/ contents",
                    ));
                    1
                }
            }
        }

        Commands::Ci {
            project_dir,
            pr_comment,
            fail_on,
            no_drift_check,
            changed_only,
        } => match ci_check::ci_check(&project_dir, !no_drift_check, changed_only) {
            Ok(result) => {
                if pr_comment {
                    println!("{}", ci_check::format_pr_comment(&result));
                } else if json_mode {
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
                    println!(
                        "Whetstone: [{}] {} (score: {}/100)",
                        s.to_uppercase(),
                        label,
                        score
                    );
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

        Commands::Eval {
            action,
            project_dir,
            collect,
            deterministic_only,
            lang,
            dry_run,
        } => {
            let project_path = Path::new(&project_dir);
            let lang_filter = lang.as_deref();

            let result = match action.as_str() {
                "generate" => eval::generate_eval_definitions(project_path, lang_filter, dry_run),
                "run" => eval::run_evals(project_path, lang_filter, collect, deterministic_only),
                "calibrate" => eval::calibrate(project_path, lang_filter, collect),
                _ => {
                    eprintln!("Unknown eval action: {action}. Use: generate, run, or calibrate");
                    return 1;
                }
            };

            match result {
                Ok(result) => {
                    output::print_json(&result);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh eval --help"));
                    1
                }
            }
        }

        Commands::Refresh { project_dir, check } => {
            let project_path = Path::new(&project_dir);

            // Run doctor with changed-only + refresh to detect drift and re-resolve
            let result = doctor::doctor(doctor::DoctorOptions {
                project_dir: project_path,
                skip_dev: true,
                json_mode,
                deps_filter: None,
                verbose: false,
                changed_only: true,
                refresh: true,
                resume: false,
                max_deps: None,
                ready_only: false,
                workers: None,
                full_run: false,
                trigger: "refresh",
            });

            match result {
                Ok(mut result) => {
                    // Write the refresh diff artifact and use its drift_count as authoritative.
                    let drift_count =
                        match crate::handoff::write_refresh_diff(project_path, &result) {
                            Ok((path, dc)) => {
                                result["refresh_diff"] = serde_json::json!({
                                    "path": path.display().to_string(),
                                    "drift_count": dc,
                                });
                                dc
                            }
                            Err(e) => {
                                eprintln!("Warning: failed to write refresh diff: {e}");
                                result
                                    .get("scan")
                                    .and_then(|s| s.get("drift_count"))
                                    .or_else(|| {
                                        result.get("summary").and_then(|s| s.get("drift_count"))
                                    })
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                            }
                        };

                    if json_mode {
                        output::print_json(&result);
                    } else if drift_count == 0 {
                        println!("No dependency drift detected. Rules are current.");
                        println!("Next: wh status");
                    } else {
                        println!("{drift_count} dependencies re-resolved.");
                        println!(
                            "Diff: whetstone/.state/refresh-diff.json (schema: references/handoff-schema.md)"
                        );
                        println!("Next: review the diff and update rules for changed deps.");
                    }

                    if check && drift_count > 0 {
                        1
                    } else {
                        0
                    }
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh doctor"));
                    1
                }
            }
        }

        Commands::Promote {
            rule_id,
            to,
            project_dir,
            keep_source,
        } => match personal::promote_rule(&project_dir, &rule_id, &to, keep_source) {
            Ok(result) => {
                output::print_json(&result);
                0
            }
            Err(e) => {
                output::print_json(&output::error_json(
                    &e.to_string(),
                    "wh promote <rule-id> --to personal|project|team",
                ));
                1
            }
        },

        Commands::Layers { project_dir, lang } => {
            let whetstone_config_exists = project_dir
                .join("whetstone")
                .join("whetstone.yaml")
                .exists()
                || project_dir.join("whetstone.yaml").exists();
            let resolved = layers::resolve_merged(
                &project_dir,
                lang.as_deref(),
                whetstone_config_exists,
                true,
                false,
            );
            let merged = resolved.merged;
            let rules_list: Vec<serde_json::Value> = merged
                .iter()
                .map(|lr| {
                    serde_json::json!({
                        "id": lr.rule.id,
                        "layer": lr.layer.as_str(),
                        "language": lr.rule.language,
                        "severity": lr.rule.severity,
                        "source_name": lr.rule.source_name,
                    })
                })
                .collect();
            let result = serde_json::json!({
                "status": "ok",
                "summary": layers::summary_from(&merged),
                "rules": rules_list,
                "warnings": resolved.warnings,
                "team_resolution": resolved.team_statuses,
                "next_command": "wh validate && wh context && wh tests",
            });
            output::print_json(&result);
            0
        }

        Commands::Review {
            action,
            project_dir,
            status,
            lang,
        } => {
            let result = match action {
                Some(ReviewAction::Show { rule_id }) => review::show(&project_dir, &rule_id),
                Some(ReviewAction::Queue) => review::queue(&project_dir),
                None => review::list(review::ReviewListOptions {
                    project_dir: &project_dir,
                    status_filter: status.as_deref(),
                    lang_filter: lang.as_deref(),
                }),
            };
            match result {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        print!("{}", review::format_list(&value));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh review --help"));
                    1
                }
            }
        }

        Commands::Apply {
            rule_id,
            project_dir,
            approve,
            deny,
            deprecate,
            supersede,
            reason,
            superseded_by,
            actor,
            batch,
            dry_run,
        } => {
            if let Some(batch_path) = batch {
                let result = review::apply_batch(&project_dir, &batch_path, dry_run);
                return match result {
                    Ok(v) => {
                        output::print_json(&v);
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "wh apply --batch <file.json>",
                        ));
                        1
                    }
                };
            }

            let rule_id = match rule_id {
                Some(id) => id,
                None => {
                    output::print_json(&output::error_json(
                        "rule_id required",
                        "wh apply <rule-id> --approve|--deny|--deprecate|--supersede",
                    ));
                    return 1;
                }
            };

            let transition = match (approve, deny, deprecate, supersede) {
                (true, false, false, false) => review::Transition::Approve,
                (false, true, false, false) => review::Transition::Deny,
                (false, false, true, false) => review::Transition::Deprecate,
                (false, false, false, true) => review::Transition::Supersede,
                _ => {
                    output::print_json(&output::error_json(
                        "pick exactly one of --approve, --deny, --deprecate, --supersede",
                        "wh apply <rule-id> --approve",
                    ));
                    return 1;
                }
            };

            match review::apply(review::ApplyOptions {
                project_dir: &project_dir,
                rule_id: &rule_id,
                transition,
                reason: reason.as_deref(),
                superseded_by: superseded_by.as_deref(),
                actor: actor.as_deref(),
                dry_run,
            }) {
                Ok(value) => {
                    output::print_json(&value);
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh apply --help"));
                    1
                }
            }
        }

        Commands::Bench {
            action,
            project_dir,
            corpus_dir,
            scenario,
            min_f1,
            check: fail_on_regress,
        } => {
            if action != "run" && action != "snapshot" {
                output::print_json(&output::error_json(
                    &format!("unknown bench action: {action}"),
                    "wh bench run|snapshot [--check]",
                ));
                return 1;
            }
            let result = bench::run(bench::BenchOptions {
                project_dir: &project_dir,
                corpus_dir: corpus_dir.as_deref(),
                scenario_filter: scenario.as_deref(),
                min_f1,
            });
            match result {
                Ok(mut value) => {
                    if action == "snapshot" {
                        match bench::snapshot(&project_dir, &value) {
                            Ok(path) => {
                                value["snapshot"] = serde_json::json!({
                                    "path": path.display().to_string(),
                                });
                            }
                            Err(e) => {
                                eprintln!("Warning: failed to write bench snapshot: {e}");
                            }
                        }
                    }
                    let failing = value
                        .get("summary")
                        .and_then(|s| s.get("failing"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        print!("{}", bench::format_human_output(&value));
                    }
                    if fail_on_regress && failing > 0 {
                        1
                    } else {
                        0
                    }
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh bench --help"));
                    1
                }
            }
        }

        Commands::Update { check, force } => match update::check_and_update(force, check) {
            Ok(result) => {
                if json_mode {
                    output::print_json(&result);
                } else {
                    let msg = result
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Done");
                    println!("{msg}");
                }
                0
            }
            Err(e) => {
                if json_mode {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check network connectivity and GitHub access",
                    ));
                } else {
                    eprintln!("Update failed: {e}");
                }
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
