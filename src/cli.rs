use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use crate::{
    approve, check, ci_check, config, detect, doctor, extract, gen, generate_context,
    generate_lint, generate_tests, output, personal, report, resolve, review, rule_authoring,
    rules, rules_query, status, triggers, update, worklist,
};

// TODO(whetstone-aww): reinstate patterns
// use crate::detect_patterns;

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
    /// Show the dependency-scoped extraction worklist (optionally filtered)
    Worklist {
        /// Filter to a single dependency name
        #[arg(long)]
        dep: Option<String>,
        /// Filter to a single language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,
    },
}

#[derive(Subcommand)]
enum ExtractAction {
    /// Submit a bundle of candidate rules to whetstone/rules/<lang>/<dep>.yaml
    Submit {
        /// Path to the candidate bundle YAML
        bundle: PathBuf,
    },
}

#[derive(Subcommand)]
enum RuleAction {
    /// Add a new rule directly (personal taste shortcut; defaults to the personal layer)
    Add {
        /// Rule id (format: `<dep>.<rule-name>`, or pass --dep with bare rule name)
        rule_id: String,

        /// Plain-English description of the rule
        #[arg(long)]
        description: String,

        /// Regex pattern that signals a violation (produces a `pattern` signal)
        #[arg(long = "match")]
        match_regex: Option<String>,

        /// Severity [must | should | may]
        #[arg(long, default_value = "should")]
        severity: String,

        /// Confidence [high | medium]
        #[arg(long, default_value = "high")]
        confidence: String,

        /// Category [migration | default | convention | breaking-change | semantic]
        #[arg(long, default_value = "convention")]
        category: String,

        /// Language [python | typescript | rust]
        #[arg(long)]
        lang: String,

        /// Documentation URL backing the rule (default: personal:// placeholder)
        #[arg(long)]
        source_url: Option<String>,

        /// Dependency name (overrides the prefix in --rule-id)
        #[arg(long)]
        dep: Option<String>,

        /// Route to the committed project layer instead of the gitignored personal layer
        #[arg(long, conflicts_with = "personal")]
        project: bool,

        /// Route to the personal layer (default)
        #[arg(long, conflicts_with = "project")]
        personal: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Edit severity / confidence on one approved rule, or bulk via --all with selectors
    Edit {
        /// Rule id to edit (omit when using --all)
        rule_id: Option<String>,

        /// Bulk edit every matching approved rule
        #[arg(long)]
        all: bool,

        /// Filter: only rules whose id starts with `<dep>.` (--all only)
        #[arg(long)]
        dep: Option<String>,

        /// Filter: only rules in this category (--all only)
        #[arg(long)]
        category: Option<String>,

        /// New severity [must | should | may]
        #[arg(long)]
        severity: Option<String>,

        /// New confidence [high | medium]
        #[arg(long)]
        confidence: Option<String>,

        /// Preview the changes without writing
        #[arg(long)]
        dry_run: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum RulesAction {
    /// Query approved rules that match the given filters
    Query {
        /// Return rules for the language inferred from this file's extension
        #[arg(long)]
        file: Option<PathBuf>,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Filter by dependency / source name
        #[arg(long)]
        dep: Option<String>,

        /// Filter by severity (must, should, may)
        #[arg(long)]
        severity: Option<String>,

        /// Layer filter: personal-only
        #[arg(long, conflicts_with = "project_only")]
        personal_only: bool,

        /// Layer filter: project-only
        #[arg(long, conflicts_with = "personal_only")]
        project_only: bool,

        /// Include full signal details and golden examples (default: summary only)
        #[arg(long)]
        full: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap from zero: detect dependencies, resolve documentation, write extraction handoff
    #[command(name = "init")]
    Init {
        /// Root directory to search for manifest files
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Only scan manifests and print detected deps — skip source resolution
        #[arg(long)]
        detect_only: bool,

        /// Compare current deps against stored versions in whetstone rules (detect-only mode)
        #[arg(long)]
        check_drift: bool,

        /// Scope: detect-only drifted deps, or bootstrap only changed/stale sources
        #[arg(long)]
        changed_only: bool,

        /// Comma-separated directory patterns to exclude (detect-only mode)
        #[arg(long)]
        exclude: Option<String>,

        /// Comma-separated directory patterns to include even if normally skipped (detect-only mode)
        #[arg(long)]
        include: Option<String>,

        /// Compare manifest fingerprints and persist dependency inventory (detect-only mode)
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

        /// Include dev dependencies in the bootstrap (default: skip)
        #[arg(long)]
        include_dev: bool,

        /// Comma-separated dependency names to target
        #[arg(long)]
        deps: Option<String>,

        /// Show full source list in report
        #[arg(long)]
        verbose: bool,

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

    /// Resolve documentation URLs and fetch content for dependencies
    #[command(name = "set-sources")]
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

        /// HTTP request timeout in seconds (overrides resolve.timeout_seconds)
        #[arg(long)]
        timeout: Option<u64>,

        /// Cache TTL in seconds (overrides resolve.cache_ttl_seconds; default 7 days)
        #[arg(long)]
        ttl: Option<u64>,

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
    #[command(name = "context")]
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

        /// Emit a one-line-per-rule bootstrap; agents use `wh rules query --file <path>` for details
        #[arg(long)]
        terse: bool,
    },

    /// Generate test files from approved rules
    #[command(name = "tests")]
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

    /// Generate context, tests, and lint configs in one chain
    #[command(name = "actions")]
    Actions {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,

        /// Emit everything under whetstone/.personal/ instead of whetstone/
        #[arg(long)]
        personal: bool,

        /// Emit terse context (one-line-per-rule bootstrap)
        #[arg(long)]
        terse: bool,
    },

    /// Generate linter configuration overlays from approved rules
    #[command(name = "lint")]
    Lint {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,

        /// Emit personal-layer lint configs into whetstone/.personal/lint/
        #[arg(long)]
        personal: bool,
    },

    /// Flip candidate rules to approved (single id or batch filters)
    Approve {
        /// Rule id to approve. Omit when using --all.
        rule_id: Option<String>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Approve every matching candidate rule.
        #[arg(long, conflicts_with = "rule_id")]
        all: bool,

        /// Restrict --all to a single dependency
        #[arg(long, requires = "all")]
        dep: Option<String>,

        /// Restrict --all to candidates with this confidence (`high`|`medium`)
        #[arg(long, requires = "all")]
        confidence: Option<String>,
    },

    /// Walk the extraction worklist or submit a candidate bundle
    Extract {
        #[command(subcommand)]
        action: Option<ExtractAction>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Filter the worklist to a single dependency name
        #[arg(long)]
        dep: Option<String>,

        /// Filter the worklist to a single language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,
    },

    /// Validate the rule schema and all rule fixtures
    #[command(name = "validate")]
    Validate {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    // TODO(whetstone-aww): reinstate patterns
    // /// Mine style patterns from transcripts, git history, and PR comments
    // Patterns { ... }
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
    #[command(name = "ci")]
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
    #[command(name = "reinit")]
    Reinit {
        /// Project directory
        #[arg(long, default_value = ".")]
        project_dir: String,

        /// Exit non-zero if drift exists (for CI)
        #[arg(long)]
        check: bool,
    },

    /// Review rules by lifecycle status (candidate / approved)
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

    /// Query rules that apply to a file / language / dep (JIT consumption)
    #[command(name = "rules")]
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },

    /// Add or edit an approved rule directly (authoring shortcut)
    #[command(name = "rule")]
    Rule {
        #[command(subcommand)]
        action: RuleAction,
    },

    /// One-page report: adherence score, top violations, drift, next actions
    #[command(name = "report")]
    Report {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Emit the GitHub-flavored PR-comment markdown (adds a tracking marker)
        #[arg(long)]
        pr_comment: bool,
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
            detect_only,
            check_drift,
            changed_only,
            exclude,
            include,
            incremental,
            personal,
            hooks,
            ci,
            schedule,
            include_dev,
            deps,
            verbose,
            refresh,
            resume,
            max_deps,
            ready_only,
            workers,
            full_run,
        } => {
            // Setup flags short-circuit everything else. They can compose — e.g.
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

            if detect_only {
                let cli_excludes: Vec<String> = exclude
                    .map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
                    .unwrap_or_default();
                let cli_includes: Vec<String> = include
                    .map(|s| s.split(',').map(|i| i.trim().to_string()).collect())
                    .unwrap_or_default();

                let do_drift = check_drift || changed_only;
                return match detect::detect_deps(
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
                                                v.get("name")
                                                    .and_then(|n| n.as_str())
                                                    .map(String::from)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();

                                if !changed.is_empty() {
                                    if let Some(deps_value) = result.get_mut("dependencies") {
                                        if let Some(arr) = deps_value.as_array() {
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
                                            *deps_value = serde_json::Value::Array(filtered);
                                        }
                                    }
                                    result["next_command"] = serde_json::json!(
                                        "Resolve changed sources: wh set-sources --changed-only"
                                    );
                                } else {
                                    result["dependencies"] = serde_json::json!([]);
                                    result["next_command"] = serde_json::json!(
                                        "No changes detected. Rules are current."
                                    );
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
                };
            }

            // Default: full bootstrap.
            let skip_dev = !include_dev;
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
                trigger: "init",
            }) {
                Ok(result) => {
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
                    }));
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

            // Precedence: explicit CLI flag > config > hardcoded default.
            let cfg = config::WhetstoneConfig::load(&project_dir);
            let effective_timeout = timeout.or(cfg.resolve.timeout_seconds).unwrap_or(15);
            let effective_ttl = ttl.or(cfg.resolve.cache_ttl_seconds).unwrap_or(604800);
            let effective_workers = workers.or(cfg.resolve.workers);

            match resolve::resolve_sources(resolve::ResolveOptions {
                deps_data: &deps_data,
                filter_deps: filter_deps.as_deref(),
                changed_only,
                project_dir: &project_dir,
                timeout: effective_timeout,
                ttl: effective_ttl,
                force_refresh,
                resume,
                retry_failed,
                workers: effective_workers,
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
            terse,
        } => {
            match generate_context::generate_context(
                &project_dir,
                formats.as_deref(),
                lang.as_deref(),
                dry_run,
                personal,
                terse,
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

        Commands::Actions {
            project_dir,
            lang,
            dry_run,
            personal,
            terse,
        } => match gen::run(&project_dir, lang.as_deref(), dry_run, personal, terse) {
            Ok(result) => {
                if json_mode {
                    output::print_json(&result);
                } else {
                    println!("wh actions: context + tests + lint generated");
                    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
                        println!("Next: {next}");
                    }
                }
                0
            }
            Err(e) => {
                output::print_json(&output::error_json(
                    &e.to_string(),
                    "wh actions runs context + tests + lint; fix the first failing generator and retry",
                ));
                1
            }
        },

        Commands::Lint {
            project_dir,
            lang,
            dry_run,
            personal,
        } => match generate_lint::generate_lint(&project_dir, lang.as_deref(), dry_run, personal) {
            Ok(result) => {
                if json_mode {
                    output::print_json(&result);
                } else if let Some(gen) = result.get("generated") {
                    if let Some(lints) = gen.get("lint_configs").and_then(|v| v.as_array()) {
                        for f in lints {
                            let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                            println!("  + {path}");
                        }
                    }
                }
                0
            }
            Err(e) => {
                output::print_json(&output::error_json(
                    &e.to_string(),
                    "Check whetstone/rules/ directory for approved rules with lint_proxy signals",
                ));
                1
            }
        },

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

        Commands::Approve {
            rule_id,
            project_dir,
            all,
            dep,
            confidence,
        } => {
            let result = match (rule_id, all) {
                (Some(id), false) => approve::approve_by_id(&project_dir, &id),
                (None, true) => {
                    approve::approve_bulk(&project_dir, dep.as_deref(), confidence.as_deref())
                }
                (None, false) => {
                    output::print_json(&output::error_json(
                        "wh approve requires a <rule-id> or --all",
                        "wh approve <rule-id> | wh approve --all [--dep <name>] [--confidence high]",
                    ));
                    return 1;
                }
                (Some(_), true) => unreachable!("clap conflicts_with guards this"),
            };
            match result {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else if let Some(count) = value.get("approved_count").and_then(|v| v.as_i64())
                    {
                        println!("wh approve: {count} candidate(s) approved");
                    } else {
                        let id = value.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("wh approve: {id} -> {action}");
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh approve --help"));
                    1
                }
            }
        }

        Commands::Extract {
            action,
            project_dir,
            dep,
            lang,
        } => {
            let result = match action {
                Some(ExtractAction::Submit { bundle }) => extract::submit(&project_dir, &bundle),
                None => extract::show_worklist(&project_dir, dep.as_deref(), lang.as_deref()),
            };
            match result {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else if value.get("wrote").is_some() {
                        let wrote = value
                            .get("wrote")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        println!("wh extract submit: wrote {wrote}");
                        if let Some(next) = value.get("next_command").and_then(|v| v.as_str()) {
                            println!("Next: {next}");
                        }
                    } else {
                        print!("{}", review::format_worklist(&value));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh extract --help"));
                    1
                }
            }
        }

        Commands::Validate { project_dir } => {
            let (report, ok) = rules::validate_schema_and_fixtures(&project_dir);
            print!("{report}");
            if ok {
                0
            } else {
                1
            }
        }

        Commands::Check {
            paths,
            project_dir,
            lang,
            rule,
            no_fail,
        } => {
            let cfg = config::WhetstoneConfig::load(&project_dir);
            let scan_paths: Vec<PathBuf> = if !paths.is_empty() {
                paths
            } else if !cfg.check.paths.is_empty() {
                cfg.check
                    .paths
                    .iter()
                    .map(|p| project_dir.join(p))
                    .collect()
            } else {
                vec![project_dir.clone()]
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
                    let fail_mode = cfg.check.fail_on.as_deref().unwrap_or("both");
                    let should_fail = !no_fail
                        && match fail_mode {
                            "none" => false,
                            "violations" => violations_count > 0,
                            "config_issues" => config_issues_count > 0,
                            _ => violations_count > 0 || config_issues_count > 0,
                        };
                    if should_fail {
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

        Commands::Reinit { project_dir, check } => {
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
                trigger: "reinit",
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
                        // Read the written refresh-diff and surface re-extraction candidates.
                        let diff_path = project_path
                            .join("whetstone")
                            .join(".state")
                            .join("refresh-diff.json");
                        if let Ok(text) = std::fs::read_to_string(&diff_path) {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let Some(cands) = v
                                    .get("re_extraction_candidates")
                                    .and_then(|c| c.as_array())
                                {
                                    if !cands.is_empty() {
                                        println!(
                                            "{} approved rule(s) may need attention:",
                                            cands.len()
                                        );
                                        for c in cands.iter().take(10) {
                                            let id = c
                                                .get("rule_id")
                                                .and_then(|s| s.as_str())
                                                .unwrap_or("?");
                                            let drift = c
                                                .get("drift_types")
                                                .and_then(|s| s.as_array())
                                                .map(|a| {
                                                    a.iter()
                                                        .filter_map(|v| v.as_str())
                                                        .collect::<Vec<_>>()
                                                        .join(",")
                                                })
                                                .unwrap_or_default();
                                            println!("  {id}  ({drift})");
                                        }
                                        if cands.len() > 10 {
                                            println!("  … +{} more", cands.len() - 10);
                                        }
                                    }
                                }
                            }
                        }
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
                    output::print_json(&output::error_json(&e.to_string(), "wh init"));
                    1
                }
            }
        }

        Commands::Review {
            action,
            project_dir,
            status,
            lang,
        } => {
            enum Render {
                List,
                Worklist,
                Show,
            }

            let (result, render) = match action {
                Some(ReviewAction::Show { rule_id }) => {
                    (review::show(&project_dir, &rule_id), Render::Show)
                }
                Some(ReviewAction::Worklist {
                    dep: wl_dep,
                    lang: wl_lang,
                }) => {
                    let res = match worklist::load(&project_dir) {
                        Ok(handoff) => {
                            let wl = handoff
                                .get("worklist")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let filtered =
                                worklist::filter(&wl, wl_dep.as_deref(), wl_lang.as_deref());
                            Ok(serde_json::json!({
                                "status": "ok",
                                "generated_at": handoff.get("generated_at"),
                                "trigger": handoff.get("trigger"),
                                "total": filtered.len(),
                                "entries": filtered,
                                "next_command": "Pick the first `ready_now` entry, extract rules, and `wh extract submit <bundle>`",
                            }))
                        }
                        Err(e) => Err(e),
                    };
                    (res, Render::Worklist)
                }
                None => (
                    review::list(review::ReviewListOptions {
                        project_dir: &project_dir,
                        status_filter: status.as_deref(),
                        lang_filter: lang.as_deref(),
                    }),
                    Render::List,
                ),
            };
            match result {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        match render {
                            Render::List | Render::Show => {
                                print!("{}", review::format_list(&value));
                            }
                            Render::Worklist => {
                                print!("{}", review::format_worklist(&value));
                            }
                        }
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh review --help"));
                    1
                }
            }
        }

        Commands::Rule { action } => match action {
            RuleAction::Add {
                rule_id,
                description,
                match_regex,
                severity,
                confidence,
                category,
                lang,
                source_url,
                dep,
                project,
                personal: _personal,
                project_dir,
            } => {
                // Default is personal unless --project is explicit.
                let personal = !project;
                let opts = rule_authoring::AddOptions {
                    rule_id: &rule_id,
                    description: &description,
                    match_regex: match_regex.as_deref(),
                    severity: &severity,
                    confidence: &confidence,
                    category: &category,
                    language: &lang,
                    source_url: source_url.as_deref(),
                    dep: dep.as_deref(),
                    personal,
                };
                match rule_authoring::add(&project_dir, opts) {
                    Ok(v) => {
                        if json_mode {
                            output::print_json(&v);
                        } else {
                            let wrote = v.get("wrote").and_then(|s| s.as_str()).unwrap_or("?");
                            let rule = v.get("rule_id").and_then(|s| s.as_str()).unwrap_or("?");
                            println!("Added {rule} → {wrote}");
                        }
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "Check inputs: rule id format, severity/confidence/category/lang values",
                        ));
                        1
                    }
                }
            }
            RuleAction::Edit {
                rule_id,
                all,
                dep,
                category,
                severity,
                confidence,
                dry_run,
                project_dir,
            } => {
                let selector = rule_authoring::EditSelector {
                    rule_id: rule_id.as_deref(),
                    all,
                    dep: dep.as_deref(),
                    category: category.as_deref(),
                };
                let mutation = rule_authoring::EditMutation {
                    severity: severity.as_deref(),
                    confidence: confidence.as_deref(),
                };
                match rule_authoring::edit(&project_dir, selector, mutation, dry_run) {
                    Ok(v) => {
                        if json_mode {
                            output::print_json(&v);
                        } else {
                            let count = v.get("count").and_then(|n| n.as_u64()).unwrap_or(0);
                            let word = if dry_run { "would change" } else { "changed" };
                            println!("{word} {count} rule(s)");
                            if let Some(items) = v.get("changed").and_then(|a| a.as_array()) {
                                for item in items {
                                    let id = item
                                        .get("rule_id")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("?");
                                    let file =
                                        item.get("file").and_then(|s| s.as_str()).unwrap_or("?");
                                    println!("  {id}  ({file})");
                                }
                            }
                        }
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "Use `wh review` to find rule ids; `wh rule edit --help` for flags",
                        ));
                        1
                    }
                }
            }
        },

        Commands::Rules { action } => match action {
            RulesAction::Query {
                file,
                lang,
                dep,
                severity,
                personal_only,
                project_only,
                full,
                project_dir,
            } => {
                let layer_filter = if personal_only {
                    rules_query::LayerFilter::PersonalOnly
                } else if project_only {
                    rules_query::LayerFilter::ProjectOnly
                } else {
                    rules_query::LayerFilter::All
                };
                let detail = if full {
                    rules_query::Detail::Full
                } else {
                    rules_query::Detail::Summary
                };

                let filters = rules_query::Filters {
                    file: file.as_deref(),
                    lang: lang.as_deref(),
                    dep: dep.as_deref(),
                    severity: severity.as_deref(),
                    layer_filter,
                };

                let result = rules_query::query(&project_dir, &filters);
                let echo = rules_query::filters_to_json(
                    file.as_deref(),
                    lang.as_deref(),
                    dep.as_deref(),
                    severity.as_deref(),
                    layer_filter,
                    detail,
                );

                if json_mode {
                    output::print_json(&rules_query::to_json(&result, detail, echo));
                } else {
                    print!("{}", rules_query::to_human(&result, detail));
                }
                0
            }
        },

        Commands::Report {
            project_dir,
            pr_comment,
        } => {
            let opts = report::ReportOptions {
                project_dir: &project_dir,
                pr_comment,
            };
            match report::build(&opts) {
                Ok(data) => {
                    // --pr-comment always emits markdown (it's the whole point);
                    // otherwise honor JSON auto-detect.
                    if pr_comment {
                        print!("{}", report::to_markdown(&data));
                    } else if json_mode {
                        output::print_json(&data);
                    } else {
                        print!("{}", report::to_markdown(&data));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "wh report composes wh status + wh check; fix the underlying failure and retry",
                    ));
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
