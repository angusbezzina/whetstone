use clap::{Args, Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    approve, check, ci_check, config, debt, detect, doctor, extract, gen, generate_context,
    generate_lint, generate_tests, output, personal, report, resolve, review, rule_authoring,
    rules, rules_query, source_mgmt, status, triggers, tui, update, worklist,
};

const WORKLIST_MAX_ENTRIES: usize = 200;

const TAXONOMY_HELP: &str = "Core workflow:
  whetstone init             Bootstrap from zero
  whetstone sources list     Review source fetch health + confidence
  whetstone rules worklist   Move source findings into candidate rules
  whetstone scan             Validate approved rules against violations
  whetstone reinit           Refresh changed dependencies/docs
  whetstone status           Health + adherence summary
  whetstone debt             Deterministic debt hotspots
  whetstone actions all      Generate context + tests + lint

Management:
  whetstone rules ...        list | show | query | add | edit | remove | approve | worklist
  whetstone sources ...      list | add | edit | remove | verify
  whetstone config ...       show | validate

Maintenance:
  whetstone extract          Draft or submit candidate rules
  whetstone rules approve    Approve candidate rules
  whetstone validate         Validate rule files
  whetstone update           Update whetstone

Reporting:
  whetstone status --report [--pr-comment]

Compatibility notes:
  Some older top-level commands remain available but are hidden from this help:
  set-sources, context, tests, lint, ci, review, report.
  Older spellings still work as aliases: check -> scan, rule -> rules,
  source -> sources, fetch -> verify.

Agent mode:
  Pass --json for machine-readable output. Bare interactive TTY runs default to the TUI.";

#[derive(Parser)]
#[command(
    name = "whetstone",
    about = "Whetstone \u{2014} sharpen the tools that write your code.",
    version,
    after_help = TAXONOMY_HELP
)]
struct Cli {
    /// Output machine-readable JSON instead of human-friendly text
    #[arg(long, global = true)]
    json: bool,

    /// Project root directory (only used when no subcommand is given — launches TUI)
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
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

#[derive(Args, Clone)]
struct ActionArgs {
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
}

#[derive(Args, Clone)]
struct ActionNoTerseArgs {
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
}

#[derive(Subcommand)]
enum ActionsAction {
    /// Generate context, tests, and lint configs in one chain
    All(ActionArgs),
    /// Generate agent context files from approved rules
    Context(ActionArgs),
    /// Generate linter configuration overlays from approved rules
    Lint(ActionNoTerseArgs),
    /// Generate test files from approved rules
    #[command(name = "test", alias = "tests")]
    Test(ActionNoTerseArgs),
}

#[derive(Subcommand)]
enum SourceAction {
    /// Subscribe to a custom rule source (blog / wiki / llms.txt / internal doc)
    Add {
        /// URL of the source (http:// or https://)
        url: String,

        /// Short name for the source (defaults to the URL)
        #[arg(long)]
        name: Option<String>,

        /// Language scope (python | typescript | rust | any)
        #[arg(long)]
        lang: Option<String>,

        /// Source kind (blog | official_docs | team_guide | community | custom)
        #[arg(long)]
        kind: Option<String>,

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
    /// Edit a subscribed custom source in place
    Edit {
        /// URL or name of the source to edit
        target: String,

        /// Replace the source URL
        #[arg(long)]
        url: Option<String>,

        /// Replace the short name
        #[arg(long)]
        name: Option<String>,

        /// Replace the language scope (python | typescript | rust | any)
        #[arg(long)]
        lang: Option<String>,

        /// Replace the source kind
        #[arg(long)]
        kind: Option<String>,

        /// Route to the committed project layer instead of personal
        #[arg(long, conflicts_with = "personal")]
        project: bool,

        /// Route to the personal layer (default)
        #[arg(long, conflicts_with = "project")]
        personal: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Show all subscribed custom sources across both layers
    List {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Remove a subscription (matches by URL or name). Flags citing rules.
    Remove {
        /// URL or name of the source to remove
        target: String,

        /// Route to the committed project layer instead of personal
        #[arg(long, conflicts_with = "personal")]
        project: bool,

        /// Route to the personal layer (default)
        #[arg(long, conflicts_with = "project")]
        personal: bool,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Verify one subscribed source by re-fetching it without a full wh reinit
    #[command(name = "verify", alias = "fetch")]
    Verify {
        /// URL or name of the source to verify
        target: String,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show the effective config stack, active packs, and per-key provenance
    Show {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Validate config files and imported packs
    Validate {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum RulesAction {
    /// List rules by lifecycle status (candidate / approved)
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Show full context for a single rule
    Show {
        /// Rule id to inspect
        rule_id: String,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
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

        /// Structured lint binding tool [ruff | biome | clippy]
        #[arg(long)]
        lint_tool: Option<String>,

        /// Structured lint binding code (e.g. B006, suspicious/noExplicitAny)
        #[arg(long)]
        lint_code: Option<String>,

        /// Formatter tool [ruff | biome | rustfmt]
        #[arg(long)]
        formatter_tool: Option<String>,

        /// Formatter option(s) as key=value. Repeat for multiple options.
        #[arg(long = "formatter-option")]
        formatter_options: Vec<String>,

        /// Explicit test runner [pytest | vitest | cargo]
        #[arg(long)]
        test_runner: Option<String>,

        /// Path to a specific test file to bind this rule to
        #[arg(long)]
        test_path: Option<String>,

        /// Optional test selector within the bound test file
        #[arg(long)]
        test_selector: Option<String>,

        /// Validator adapter name for adapter-backed enforcement (e.g. command, lint_rule, linked_test)
        #[arg(long)]
        validator_adapter: Option<String>,

        /// Validator rule / policy identifier for adapter-backed enforcement
        #[arg(long)]
        validator_rule: Option<String>,

        /// Validator config option(s) as key=value. Repeat for multiple options.
        #[arg(long = "validator-config")]
        validator_configs: Vec<String>,

        /// Explicitly create an advisory rule with no concrete enforcement binding
        #[arg(long)]
        advisory: bool,

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
    /// Remove a rule by id
    Remove {
        /// Rule id to remove
        rule_id: String,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
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
    /// Approve candidate rules (single id or batch filters)
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
    /// Review rules by lifecycle status (candidate / approved)
    Review {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,

        /// Filter by language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
    /// Show the extraction worklist through the rule workflow lens
    Worklist {
        /// Filter to a single dependency name
        #[arg(long)]
        dep: Option<String>,

        /// Filter to a single language (python, typescript, rust)
        #[arg(long)]
        lang: Option<String>,

        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
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
    #[command(name = "set-sources", hide = true)]
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

    /// Project health summary and drift detection (`--report` for markdown narrative)
    Status {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Output only score and label
        #[arg(long)]
        score: bool,

        /// Render the one-page report instead of the status summary
        #[arg(long)]
        report: bool,

        /// When used with --report, emit the PR-comment markdown variant
        #[arg(long, requires = "report")]
        pr_comment: bool,

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
    #[command(name = "context", hide = true)]
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
    #[command(name = "tests", hide = true)]
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

    /// Generate context, tests, and lint configs via explicit subcommands
    #[command(name = "actions")]
    Actions {
        #[command(subcommand)]
        action: ActionsAction,
    },

    /// Generate linter configuration overlays from approved rules
    #[command(name = "lint", hide = true)]
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

    /// Scan source files for rule violations using tree-sitter and regex signals
    #[command(name = "scan", alias = "check")]
    Scan {
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
    #[command(name = "ci", hide = true)]
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
    #[command(hide = true)]
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

    /// Manage rules, approvals, and JIT rule lookup
    #[command(name = "rules", alias = "rule")]
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },

    /// Subscribe to custom rule sources (blogs, wikis, llms.txt, internal docs)
    #[command(name = "sources", alias = "source")]
    Sources {
        #[command(subcommand)]
        action: SourceAction,
    },

    /// Inspect and validate the effective config stack and imported packs
    #[command(name = "config")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// One-page report: adherence score, top violations, drift, next actions
    #[command(name = "report", hide = true)]
    Report {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Emit the GitHub-flavored PR-comment markdown (adds a tracking marker)
        #[arg(long)]
        pr_comment: bool,
    },

    /// Surface AI-amplified technical debt hotspots (dead code, duplicates,
    /// dep hygiene, churn × violations).
    #[command(name = "debt")]
    Debt {
        /// Project root directory
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Emit a compact remediation prompt instead of the human report
        #[arg(long, conflicts_with = "beads")]
        prompt: bool,

        /// File a Beads epic + one child task per ranked hotspot
        #[arg(long)]
        beads: bool,

        /// Cap the number of ranked hotspots (default: 20)
        #[arg(long, default_value = "20")]
        top: usize,

        /// Minimum confidence filter: high | medium (default: medium)
        #[arg(long, default_value = "medium")]
        min_confidence: String,

        /// Churn window for the hotspot detector (`90d` or plain day count)
        #[arg(long = "since", alias = "since-days", default_value = "90d", value_parser = parse_since_days)]
        since_days: u32,
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
    let human_tui_mode =
        !cli.json && tui::stdout_is_tty() && std::env::var_os("WHETSTONE_NO_TUI").is_none();
    let json_mode = cli.json || (!human_tui_mode && output::is_piped());

    // Bare `wh` on a TTY → launch the interactive TUI dashboard.
    // Bare `wh` piped or redirected → print help (so scripts don't hang).
    let command = match cli.command {
        Some(c) => c,
        None => {
            if tui::stdout_is_tty() && !json_mode {
                return match tui::run(&cli.project_dir) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("whetstone: tui exited with error: {e}");
                        1
                    }
                };
            } else {
                use clap::CommandFactory;
                let _ = Cli::command().print_help();
                println!();
                return 0;
            }
        }
    };

    if human_tui_mode {
        return launch_tui_for_command(&command);
    }

    match command {
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
                                        "Resolve changed sources: wh reinit"
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
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check project directory and source resolution settings",
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
            report: status_report,
            pr_comment,
            no_drift_check,
            changed_only,
            history,
            no_snapshot,
            extraction_ready,
        } => {
            if status_report {
                return run_report_command(
                    &project_dir,
                    pr_comment,
                    json_mode,
                    "wh status --report composes wh status + wh scan; fix the underlying failure and retry",
                );
            }

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
        } => run_context_command(
            &project_dir,
            formats.as_deref(),
            lang.as_deref(),
            dry_run,
            personal,
            terse,
            json_mode,
        ),

        Commands::Actions { action } => match action {
            ActionsAction::All(args) => match gen::run(
                &args.project_dir,
                args.lang.as_deref(),
                args.dry_run,
                args.personal,
                args.terse,
            ) {
                Ok(result) => {
                    if json_mode {
                        output::print_json(&result);
                    } else {
                        println!("wh actions all: context + tests + lint generated");
                        if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
                            println!("Next: {next}");
                        }
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "wh actions all runs context + tests + lint; fix the first failing generator and retry",
                    ));
                    1
                }
            },
            ActionsAction::Context(args) => run_context_command(
                &args.project_dir,
                None,
                args.lang.as_deref(),
                args.dry_run,
                args.personal,
                args.terse,
                json_mode,
            ),
            ActionsAction::Test(args) => run_tests_command(
                &args.project_dir,
                args.lang.as_deref(),
                args.dry_run,
                args.personal,
                json_mode,
            ),
            ActionsAction::Lint(args) => run_lint_command(
                &args.project_dir,
                args.lang.as_deref(),
                args.dry_run,
                args.personal,
                json_mode,
            ),
        },

        Commands::Lint {
            project_dir,
            lang,
            dry_run,
            personal,
        } => run_lint_command(&project_dir, lang.as_deref(), dry_run, personal, json_mode),

        Commands::Tests {
            project_dir,
            lang,
            dry_run,
            personal,
        } => run_tests_command(&project_dir, lang.as_deref(), dry_run, personal, json_mode),

        Commands::Approve {
            rule_id,
            project_dir,
            all,
            dep,
            confidence,
        } => run_approve_command(
            &project_dir,
            rule_id,
            all,
            dep,
            confidence,
            json_mode,
            ApproveUiConfig {
                command_name: "wh approve",
                missing_usage_error: "wh approve requires a <rule-id> or --all",
                missing_usage_hint:
                    "wh approve <rule-id> | wh approve --all [--dep <name>] [--confidence high]",
                failure_hint: "wh approve --help",
            },
        ),

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
                        let wrote = value.get("wrote").and_then(|v| v.as_str()).unwrap_or("?");
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

        Commands::Scan {
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
                                if let Some(cands) =
                                    v.get("re_extraction_candidates").and_then(|c| c.as_array())
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
                }) => (
                    build_worklist_value(&project_dir, wl_dep.as_deref(), wl_lang.as_deref()),
                    Render::Worklist,
                ),
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

        Commands::Rules { action } => match action {
            RulesAction::List {
                status,
                lang,
                project_dir,
            }
            | RulesAction::Review {
                status,
                lang,
                project_dir,
            } => match review::list(review::ReviewListOptions {
                project_dir: &project_dir,
                status_filter: status.as_deref(),
                lang_filter: lang.as_deref(),
            }) {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        print!("{}", review::format_list(&value));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh rules list --help"));
                    1
                }
            },
            RulesAction::Show {
                rule_id,
                project_dir,
            } => match review::show(&project_dir, &rule_id) {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        print!("{}", review::format_list(&value));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh rules show --help"));
                    1
                }
            },
            RulesAction::Add {
                rule_id,
                description,
                match_regex,
                lint_tool,
                lint_code,
                formatter_tool,
                formatter_options,
                test_runner,
                test_path,
                test_selector,
                validator_adapter,
                validator_rule,
                validator_configs,
                advisory,
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
                let personal = !project;
                let enforcement = match build_rule_enforcement(RuleEnforcementInput {
                    match_regex,
                    lint_tool,
                    lint_code,
                    formatter_tool,
                    formatter_options,
                    test_runner,
                    test_path,
                    test_selector,
                    validator_adapter,
                    validator_rule,
                    validator_configs,
                    advisory,
                }) {
                    Ok(enforcement) => enforcement,
                    Err(e) => {
                        output::print_json(&output::error_json(&e.to_string(), "Choose exactly one enforcement mode for `wh rules add`"));
                        return 1;
                    }
                };
                let opts = rule_authoring::AddOptions {
                    rule_id,
                    description,
                    severity,
                    confidence,
                    category,
                    language: lang,
                    source_url,
                    dep,
                    enforcement,
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
            RulesAction::Edit {
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
                                    let id =
                                        item.get("rule_id").and_then(|s| s.as_str()).unwrap_or("?");
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
                            "Use `wh rules list` to find rule ids; `wh rules edit --help` for flags",
                        ));
                        1
                    }
                }
            }
            RulesAction::Remove {
                rule_id,
                project_dir,
            } => match rule_authoring::remove(
                &project_dir,
                rule_authoring::RemoveOptions { rule_id: &rule_id },
            ) {
                Ok(v) => {
                    if json_mode {
                        output::print_json(&v);
                    } else {
                        let file = v.get("file").and_then(|s| s.as_str()).unwrap_or("?");
                        println!("Removed {rule_id} from {file}");
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Use `wh rules list` to find the right rule id",
                    ));
                    1
                }
            },
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
            RulesAction::Approve {
                rule_id,
                project_dir,
                all,
                dep,
                confidence,
            } => run_approve_command(
                &project_dir,
                rule_id,
                all,
                dep,
                confidence,
                json_mode,
                ApproveUiConfig {
                    command_name: "wh rules approve",
                    missing_usage_error: "wh rules approve requires a <rule-id> or --all",
                    missing_usage_hint:
                        "wh rules approve <rule-id> | wh rules approve --all [--dep <name>] [--confidence high]",
                    failure_hint: "wh rules approve --help",
                },
            ),
            RulesAction::Worklist {
                dep,
                lang,
                project_dir,
            } => match build_worklist_value(&project_dir, dep.as_deref(), lang.as_deref()) {
                Ok(value) => {
                    if json_mode {
                        output::print_json(&value);
                    } else {
                        print!("{}", review::format_worklist(&value));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "wh rules worklist --help",
                    ));
                    1
                }
            },
        },

        Commands::Sources { action } => match action {
            SourceAction::Add {
                url,
                name,
                lang,
                kind,
                project,
                personal: _personal,
                project_dir,
            } => {
                let personal = !project;
                let opts = source_mgmt::AddOptions {
                    url: &url,
                    name: name.as_deref(),
                    language: lang.as_deref(),
                    source_kind: kind.as_deref(),
                    personal,
                };
                match source_mgmt::add(&project_dir, opts) {
                    Ok(v) => {
                        if json_mode {
                            output::print_json(&v);
                        } else {
                            let wrote = v.get("wrote").and_then(|s| s.as_str()).unwrap_or("?");
                            let url_out = v.get("url").and_then(|s| s.as_str()).unwrap_or("?");
                            println!("Subscribed {url_out} → {wrote}");
                            println!("Next: wh sources verify {url_out}");
                        }
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "Check URL format (http:// or https://) and whether it's already subscribed",
                        ));
                        1
                    }
                }
            }
            SourceAction::Edit {
                target,
                url,
                name,
                lang,
                kind,
                project,
                personal: _personal,
                project_dir,
            } => {
                let personal = !project;
                let opts = source_mgmt::EditOptions {
                    target: &target,
                    url: url.as_deref(),
                    name: name.as_deref(),
                    language: lang.as_deref(),
                    source_kind: kind.as_deref(),
                    personal,
                };
                match source_mgmt::edit(&project_dir, opts) {
                    Ok(v) => {
                        if json_mode {
                            output::print_json(&v);
                        } else {
                            let wrote = v.get("wrote").and_then(|s| s.as_str()).unwrap_or("?");
                            println!("Updated {target} → {wrote}");
                        }
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "Use `wh sources list` to find the right target",
                        ));
                        1
                    }
                }
            }
            SourceAction::List { project_dir } => match source_mgmt::list(&project_dir) {
                Ok(v) => {
                    if json_mode {
                        output::print_json(&v);
                    } else {
                        print!("{}", source_mgmt::format_list_human(&v));
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(&e.to_string(), "wh sources list"));
                    1
                }
            },
            SourceAction::Remove {
                target,
                project,
                personal: _personal,
                project_dir,
            } => {
                let personal = !project;
                let opts = source_mgmt::RemoveOptions {
                    target: &target,
                    personal,
                };
                match source_mgmt::remove(&project_dir, opts) {
                    Ok(v) => {
                        if json_mode {
                            output::print_json(&v);
                        } else {
                            let url = v
                                .get("removed_url")
                                .and_then(|s| s.as_str())
                                .unwrap_or(&target);
                            let wrote = v.get("wrote").and_then(|s| s.as_str()).unwrap_or("?");
                            println!("Unsubscribed {url} ← {wrote}");
                            if let Some(citers) =
                                v.get("citing_rule_ids").and_then(|a| a.as_array())
                            {
                                if !citers.is_empty() {
                                    println!("{} approved rule(s) cite this source:", citers.len());
                                    for c in citers.iter().take(10) {
                                        let id = c
                                            .get("rule_id")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("?");
                                        println!("  {id}");
                                    }
                                    println!(
                                        "Next: `wh rules edit <id>` or remove the rule if the source is gone for good"
                                    );
                                }
                            }
                        }
                        0
                    }
                    Err(e) => {
                        output::print_json(&output::error_json(
                            &e.to_string(),
                            "Use `wh sources list` to find the right target",
                        ));
                        1
                    }
                }
            }
            SourceAction::Verify {
                target,
                project_dir,
            } => match source_mgmt::fetch(&project_dir, &target) {
                Ok(v) => {
                    if json_mode {
                        output::print_json(&v);
                    } else {
                        let fetched = v.get("fetched").and_then(|n| n.as_u64()).unwrap_or(0);
                        println!("Verified {fetched} source(s)");
                        if let Some(arr) = v.get("sources").and_then(|a| a.as_array()) {
                            for s in arr {
                                let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("?");
                                let bytes = s
                                    .get("content_bytes")
                                    .and_then(|x| x.as_u64())
                                    .map(|c| c as usize)
                                    .unwrap_or(0);
                                println!("  {name}  ({bytes} bytes)");
                            }
                        }
                    }
                    0
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Check the URL resolves and `wh sources list` shows it as subscribed",
                    ));
                    1
                }
            },
        },

        Commands::Config { action } => match action {
            ConfigAction::Show { project_dir } => {
                let snapshot = config::WhetstoneConfig::load_full(&project_dir);
                if json_mode {
                    output::print_json(&snapshot.to_json());
                } else {
                    print!("{}", config::format_snapshot_human(&snapshot));
                }
                if snapshot.has_errors() {
                    1
                } else {
                    0
                }
            }
            ConfigAction::Validate { project_dir } => {
                let snapshot = config::WhetstoneConfig::load_full(&project_dir);
                if json_mode {
                    let mut data = snapshot.to_json();
                    data["valid"] = serde_json::Value::Bool(!snapshot.has_errors());
                    output::print_json(&data);
                } else {
                    print!("{}", config::format_snapshot_human(&snapshot));
                    if snapshot.has_errors() {
                        println!("Config validation failed.");
                    } else {
                        println!("Config validation passed.");
                    }
                }
                if snapshot.has_errors() {
                    1
                } else {
                    0
                }
            }
        },

        Commands::Report {
            project_dir,
            pr_comment,
        } => run_report_command(
            &project_dir,
            pr_comment,
            json_mode,
            "wh report composes wh status + wh scan; fix the underlying failure and retry",
        ),

        Commands::Debt {
            project_dir,
            prompt,
            beads,
            top,
            min_confidence,
            since_days,
        } => {
            let min_conf = match min_confidence.as_str() {
                "high" => debt::types::Confidence::High,
                "medium" | "med" => debt::types::Confidence::Medium,
                other => {
                    output::print_json(&output::error_json(
                        &format!("invalid --min-confidence: {other}"),
                        "Use --min-confidence=high or --min-confidence=medium",
                    ));
                    return 1;
                }
            };
            let opts = debt::DebtOptions {
                project_dir: project_dir.clone(),
                top,
                min_confidence: min_conf,
                since_days,
            };
            match debt::run(&opts) {
                Ok(report) => {
                    // Explicit mode flags take precedence over auto-piped JSON —
                    // `wh debt --prompt` on a pipe still emits the prompt, not JSON.
                    if prompt {
                        print!("{}", debt::output::format_prompt(&report));
                        0
                    } else if beads {
                        match debt::beads::file(&report, &project_dir) {
                            Ok(filed) => {
                                if json_mode {
                                    output::print_json(
                                        &serde_json::to_value(&filed).unwrap_or_default(),
                                    );
                                } else {
                                    println!("{}", filed.message);
                                    if let Some(epic_id) = filed.epic_id {
                                        println!("Epic: {epic_id}");
                                    }
                                    if !filed.task_ids.is_empty() {
                                        println!("Tasks: {}", filed.task_ids.join(", "));
                                    }
                                }
                                0
                            }
                            Err(e) => {
                                output::print_json(&output::error_json(
                                    &e.to_string(),
                                    "Ensure bd is installed and the repo is initialized with beads before using --beads",
                                ));
                                1
                            }
                        }
                    } else if json_mode {
                        output::print_json(&serde_json::to_value(&report).unwrap_or_default());
                        0
                    } else {
                        println!("{}", debt::output::format_human(&report));
                        0
                    }
                }
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "wh debt needs a readable project dir with at least a manifest or source tree",
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

fn parse_since_days(raw: &str) -> Result<u32, String> {
    let trimmed = raw.trim();
    let digits = trimmed.strip_suffix('d').unwrap_or(trimmed);
    digits
        .parse::<u32>()
        .map_err(|_| format!("invalid churn window `{raw}`; use `90d` or a plain day count"))
}

fn run_report_command(project_dir: &Path, pr_comment: bool, json_mode: bool, hint: &str) -> i32 {
    let opts = report::ReportOptions {
        project_dir,
        pr_comment,
    };
    match report::build(&opts) {
        Ok(mut data) => {
            let markdown = report::to_markdown(&data);
            let path = match report::write_markdown_report(project_dir, &markdown) {
                Ok(path) => path,
                Err(e) => {
                    output::print_json(&output::error_json(
                        &e.to_string(),
                        "Whetstone could not write whetstone/report.md; check filesystem permissions and retry",
                    ));
                    return 1;
                }
            };
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "report_path".to_string(),
                    serde_json::Value::String(path.display().to_string()),
                );
            }
            if pr_comment {
                print!("{}", markdown);
            } else if json_mode {
                output::print_json(&data);
            } else {
                println!("Report written to {}", path.display());
            }
            0
        }
        Err(e) => {
            output::print_json(&output::error_json(&e.to_string(), hint));
            1
        }
    }
}

fn run_context_command(
    project_dir: &Path,
    formats: Option<&str>,
    lang: Option<&str>,
    dry_run: bool,
    personal: bool,
    terse: bool,
    json_mode: bool,
) -> i32 {
    match generate_context::generate_context(project_dir, formats, lang, dry_run, personal, terse) {
        Ok(result) => {
            if json_mode {
                output::print_json(&result);
            } else {
                let gen = result.get("generated").and_then(|v| v.as_array());
                if let Some(files) = gen {
                    for f in files {
                        let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                        let lines = f.get("lines").and_then(|v| v.as_i64()).unwrap_or(0);
                        let dry = if f.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false) {
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

fn run_lint_command(
    project_dir: &Path,
    lang: Option<&str>,
    dry_run: bool,
    personal: bool,
    json_mode: bool,
) -> i32 {
    match generate_lint::generate_lint(project_dir, lang, dry_run, personal) {
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
                if let Some(formatters) = gen.get("formatter_configs").and_then(|v| v.as_array()) {
                    for f in formatters {
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
    }
}

fn run_tests_command(
    project_dir: &Path,
    lang: Option<&str>,
    dry_run: bool,
    personal: bool,
    json_mode: bool,
) -> i32 {
    match generate_tests::generate_tests(project_dir, lang, dry_run, personal) {
        Ok(result) => {
            if json_mode {
                output::print_json(&result);
            } else if let Some(gen) = result.get("generated") {
                if let Some(tests) = gen.get("tests").and_then(|v| v.as_array()) {
                    for f in tests {
                        let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("  + {path}");
                    }
                }
                if let Some(linked) = gen.get("linked_tests").and_then(|v| v.as_array()) {
                    for binding in linked {
                        let runner = binding
                            .get("runner")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let path = binding.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                        let selector = binding
                            .get("selector")
                            .and_then(|v| v.as_str())
                            .map(|selector| format!(" :: {selector}"))
                            .unwrap_or_default();
                        println!("  ↳ linked test: {runner} {path}{selector}");
                    }
                }
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
                "Check whetstone/rules/ directory for approved rules",
            ));
            1
        }
    }
}

struct ApproveUiConfig<'a> {
    command_name: &'a str,
    missing_usage_error: &'a str,
    missing_usage_hint: &'a str,
    failure_hint: &'a str,
}

fn run_approve_command(
    project_dir: &Path,
    rule_id: Option<String>,
    all: bool,
    dep: Option<String>,
    confidence: Option<String>,
    json_mode: bool,
    ui: ApproveUiConfig<'_>,
) -> i32 {
    let result = match (rule_id, all) {
        (Some(id), false) => approve::approve_by_id(project_dir, &id),
        (None, true) => approve::approve_bulk(project_dir, dep.as_deref(), confidence.as_deref()),
        (None, false) => {
            output::print_json(&output::error_json(
                ui.missing_usage_error,
                ui.missing_usage_hint,
            ));
            return 1;
        }
        (Some(_), true) => unreachable!("clap conflicts_with guards this"),
    };

    match result {
        Ok(value) => {
            if json_mode {
                output::print_json(&value);
            } else if let Some(count) = value.get("approved_count").and_then(|v| v.as_i64()) {
                println!("{}: {count} candidate(s) approved", ui.command_name);
            } else {
                let id = value.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
                let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                println!("{}: {id} -> {action}", ui.command_name);
            }
            0
        }
        Err(e) => {
            output::print_json(&output::error_json(&e.to_string(), ui.failure_hint));
            1
        }
    }
}

fn worklist_sort_key(entry: &serde_json::Value) -> (String, String, String, String) {
    let bucket = entry
        .get("priority_bucket")
        .or_else(|| entry.get("priority"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let lang = entry
        .get("lang")
        .or_else(|| entry.get("language"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dep = entry
        .get("dep")
        .or_else(|| entry.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id = entry
        .get("id")
        .or_else(|| entry.get("rule_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    (bucket, lang, dep, id)
}

fn build_worklist_value(
    project_dir: &Path,
    dep: Option<&str>,
    lang: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let handoff = worklist::load(project_dir)?;
    let mut filtered = handoff
        .get("worklist")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    filtered = worklist::filter(&filtered, dep, lang);
    backfill_worklist_languages(project_dir, &mut filtered);
    filtered.sort_by_key(worklist_sort_key);

    let total = filtered.len();
    let truncated = total > WORKLIST_MAX_ENTRIES;
    if truncated {
        filtered.truncate(WORKLIST_MAX_ENTRIES);
    }

    Ok(serde_json::json!({
        "status": "ok",
        "generated_at": handoff.get("generated_at"),
        "trigger": handoff.get("trigger"),
        "handoff_path": project_dir.join("whetstone").join(".state").join("extraction-handoff.json").display().to_string(),
        "total": total,
        "returned": filtered.len(),
        "truncated": truncated,
        "max_entries": WORKLIST_MAX_ENTRIES,
        "entries": filtered,
        "next_command": "Pick the first `ready_now` entry, extract rules, and `wh extract submit <bundle>`",
    }))
}

fn backfill_worklist_languages(project_dir: &Path, entries: &mut [serde_json::Value]) {
    let mut sm = crate::state::StateManager::new(project_dir);
    sm.load_all();
    let deps = sm.inventory.all_deps();
    let mut by_name: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for dep in deps {
        let Some(name) = dep.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(language) = dep.get("language").and_then(|v| v.as_str()) else {
            continue;
        };
        by_name
            .entry(name.to_string())
            .or_default()
            .insert(language.to_string());
    }

    for entry in entries.iter_mut() {
        let missing_lang = entry
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if !missing_lang {
            continue;
        }
        let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(langs) = by_name.get(name) {
            if langs.len() == 1 {
                if let Some(language) = langs.iter().next() {
                    entry["language"] = serde_json::Value::String(language.clone());
                }
            }
        }
    }
}

fn launch_tui_for_command(command: &Commands) -> i32 {
    let project_dir = project_dir_for_command(command);
    let mut args = vec![std::ffi::OsString::from("--json")];
    args.extend(std::env::args_os().skip(1));

    let output = match std::process::Command::new(std::env::current_exe().unwrap_or_default())
        .args(args)
        .env("WHETSTONE_NO_TUI", "1")
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return launch_tui_result(
                &project_dir,
                command_title(command),
                format!("Failed to run command in background for TUI mode:\n\n{e}"),
            )
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let body = render_command_body(&stdout, &stderr);
    let success = output.status.success();

    if success {
        if let Some(screen) = success_screen_for_command(command) {
            launch_tui_screen(&project_dir, screen)
        } else {
            launch_tui_result(&project_dir, command_title(command), body)
        }
    } else {
        launch_tui_result(&project_dir, command_title(command), body)
    }
}

fn render_command_body(stdout: &str, stderr: &str) -> String {
    if !stdout.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) {
            return serde_json::to_string_pretty(&v).unwrap_or_else(|_| stdout.to_string());
        }
        return stdout.to_string();
    }
    if !stderr.trim().is_empty() {
        return stderr.to_string();
    }
    "Command completed with no textual output.".to_string()
}

fn launch_tui_result(project_dir: &Path, title: &str, body: String) -> i32 {
    match tui::run_with_target(
        project_dir,
        tui::LaunchTarget::Result {
            title: title.to_string(),
            body,
        },
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("whetstone: tui exited with error: {e}");
            1
        }
    }
}

fn launch_tui_screen(project_dir: &Path, screen: tui::msg::Screen) -> i32 {
    match tui::run_with_target(project_dir, tui::LaunchTarget::Screen(screen)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("whetstone: tui exited with error: {e}");
            1
        }
    }
}

fn command_title(command: &Commands) -> &'static str {
    match command {
        Commands::Init { .. } => "INIT",
        Commands::SetSources { .. } => "SET-SOURCES",
        Commands::Status { .. } => "STATUS",
        Commands::Context { .. } => "CONTEXT",
        Commands::Tests { .. } => "TESTS",
        Commands::Actions { .. } => "ACTIONS",
        Commands::Lint { .. } => "LINT",
        Commands::Approve { .. } => "APPROVE",
        Commands::Extract { .. } => "EXTRACT",
        Commands::Validate { .. } => "VALIDATE",
        Commands::Scan { .. } => "SCAN",
        Commands::Ci { .. } => "CI",
        Commands::Reinit { .. } => "REINIT",
        Commands::Review { .. } => "REVIEW",
        Commands::Rules { .. } => "RULES",
        Commands::Sources { .. } => "SOURCES",
        Commands::Config { .. } => "CONFIG",
        Commands::Report { .. } => "REPORT",
        Commands::Debt { .. } => "DEBT",
        Commands::Update { .. } => "UPDATE",
    }
}

fn success_screen_for_command(command: &Commands) -> Option<tui::msg::Screen> {
    use tui::msg::Screen;
    match command {
        Commands::Init {
            detect_only,
            personal,
            hooks,
            ci,
            ..
        } if !detect_only && !personal && !hooks && !ci => Some(Screen::Sources),
        Commands::SetSources { .. } => Some(Screen::Sources),
        Commands::Status {
            report,
            score,
            history,
            extraction_ready,
            ..
        } => {
            if *report || *score || *history || *extraction_ready {
                None
            } else {
                Some(Screen::Dashboard)
            }
        }
        Commands::Extract { action: None, .. } => Some(Screen::Sources),
        Commands::Scan { .. } => Some(Screen::Check),
        Commands::Reinit { .. } => Some(Screen::Dashboard),
        Commands::Report { .. } => None,
        Commands::Debt { prompt, beads, .. } if !prompt && !beads => Some(Screen::Debt),
        Commands::Sources { .. } => Some(Screen::Sources),
        Commands::Config { .. } => None,
        Commands::Approve { .. } => Some(Screen::Rules),
        Commands::Rules { action } => match action {
            RulesAction::Add { .. }
            | RulesAction::Edit { .. }
            | RulesAction::Remove { .. }
            | RulesAction::Approve { .. }
            | RulesAction::List { .. }
            | RulesAction::Review { .. }
            | RulesAction::Show { .. } => Some(Screen::Rules),
            RulesAction::Worklist { .. } => Some(Screen::Sources),
            _ => None,
        },
        Commands::Review {
            action: Some(ReviewAction::Worklist { .. }),
            ..
        } => Some(Screen::Sources),
        _ => None,
    }
}

fn project_dir_for_command(command: &Commands) -> PathBuf {
    match command {
        Commands::Init { project_dir, .. }
        | Commands::SetSources { project_dir, .. }
        | Commands::Status { project_dir, .. }
        | Commands::Context { project_dir, .. }
        | Commands::Tests { project_dir, .. }
        | Commands::Lint { project_dir, .. }
        | Commands::Approve { project_dir, .. }
        | Commands::Extract { project_dir, .. }
        | Commands::Validate { project_dir, .. }
        | Commands::Ci { project_dir, .. }
        | Commands::Review { project_dir, .. }
        | Commands::Report { project_dir, .. }
        | Commands::Debt { project_dir, .. } => project_dir.clone(),
        Commands::Actions { action } => match action {
            ActionsAction::All(args) | ActionsAction::Context(args) => args.project_dir.clone(),
            ActionsAction::Lint(args) | ActionsAction::Test(args) => args.project_dir.clone(),
        },
        Commands::Scan { project_dir, .. } => project_dir.clone(),
        Commands::Rules { action } => match action {
            RulesAction::List { project_dir, .. }
            | RulesAction::Show { project_dir, .. }
            | RulesAction::Add { project_dir, .. }
            | RulesAction::Edit { project_dir, .. }
            | RulesAction::Remove { project_dir, .. }
            | RulesAction::Query { project_dir, .. }
            | RulesAction::Approve { project_dir, .. }
            | RulesAction::Review { project_dir, .. }
            | RulesAction::Worklist { project_dir, .. } => project_dir.clone(),
        },
        Commands::Sources { action } => match action {
            SourceAction::Add { project_dir, .. }
            | SourceAction::Edit { project_dir, .. }
            | SourceAction::List { project_dir }
            | SourceAction::Remove { project_dir, .. }
            | SourceAction::Verify { project_dir, .. } => project_dir.clone(),
        },
        Commands::Config { action } => match action {
            ConfigAction::Show { project_dir } | ConfigAction::Validate { project_dir } => {
                project_dir.clone()
            }
        },
        Commands::Reinit { project_dir, .. } => PathBuf::from(project_dir),
        Commands::Update { .. } => PathBuf::from("."),
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

struct RuleEnforcementInput {
    match_regex: Option<String>,
    lint_tool: Option<String>,
    lint_code: Option<String>,
    formatter_tool: Option<String>,
    formatter_options: Vec<String>,
    test_runner: Option<String>,
    test_path: Option<String>,
    test_selector: Option<String>,
    validator_adapter: Option<String>,
    validator_rule: Option<String>,
    validator_configs: Vec<String>,
    advisory: bool,
}

fn build_rule_enforcement(
    input: RuleEnforcementInput,
) -> anyhow::Result<rule_authoring::EnforcementMode> {
    let RuleEnforcementInput {
        match_regex,
        lint_tool,
        lint_code,
        formatter_tool,
        formatter_options,
        test_runner,
        test_path,
        test_selector,
        validator_adapter,
        validator_rule,
        validator_configs,
        advisory,
    } = input;
    let mut chosen = Vec::new();
    if match_regex.is_some() {
        chosen.push("pattern");
    }
    if lint_tool.is_some() || lint_code.is_some() {
        chosen.push("lint");
    }
    if formatter_tool.is_some() || !formatter_options.is_empty() {
        chosen.push("formatter");
    }
    if test_runner.is_some() || test_path.is_some() || test_selector.is_some() {
        chosen.push("test");
    }
    if validator_adapter.is_some() || validator_rule.is_some() || !validator_configs.is_empty() {
        chosen.push("validator");
    }
    if advisory {
        chosen.push("advisory");
    }

    if chosen.len() != 1 {
        return Err(anyhow::anyhow!(
            "choose exactly one enforcement mode: --match, --lint-tool/--lint-code, --formatter-tool/--formatter-option, --test-runner/--test-path, --validator-adapter/--validator-rule, or --advisory"
        ));
    }

    if let Some(regex) = match_regex {
        return Ok(rule_authoring::EnforcementMode::Pattern { regex });
    }

    if lint_tool.is_some() || lint_code.is_some() {
        let tool = lint_tool.ok_or_else(|| anyhow::anyhow!("--lint-tool is required"))?;
        let code = lint_code.ok_or_else(|| anyhow::anyhow!("--lint-code is required"))?;
        return Ok(rule_authoring::EnforcementMode::Lint { tool, code });
    }

    if formatter_tool.is_some() || !formatter_options.is_empty() {
        let tool = formatter_tool.ok_or_else(|| anyhow::anyhow!("--formatter-tool is required"))?;
        let options = parse_formatter_options(&formatter_options)?;
        return Ok(rule_authoring::EnforcementMode::Formatter { tool, options });
    }

    if test_runner.is_some() || test_path.is_some() || test_selector.is_some() {
        let runner = test_runner.ok_or_else(|| anyhow::anyhow!("--test-runner is required"))?;
        let path = test_path.ok_or_else(|| anyhow::anyhow!("--test-path is required"))?;
        return Ok(rule_authoring::EnforcementMode::Test {
            runner,
            path,
            selector: test_selector,
        });
    }

    if validator_adapter.is_some() || validator_rule.is_some() || !validator_configs.is_empty() {
        let adapter =
            validator_adapter.ok_or_else(|| anyhow::anyhow!("--validator-adapter is required"))?;
        let rule = validator_rule.ok_or_else(|| anyhow::anyhow!("--validator-rule is required"))?;
        let config = parse_validator_options(&validator_configs)?;
        return Ok(rule_authoring::EnforcementMode::Validator {
            adapter,
            rule,
            config,
        });
    }

    Ok(rule_authoring::EnforcementMode::Advisory)
}

fn parse_formatter_options(
    entries: &[String],
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    parse_key_value_options(entries, "formatter options")
}

fn parse_validator_options(
    entries: &[String],
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    parse_key_value_options(entries, "validator config")
}

fn parse_key_value_options(
    entries: &[String],
    label: &str,
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    let mut options = BTreeMap::new();
    for entry in entries {
        let (key, raw) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("{label} must use key=value syntax (got `{entry}`)"))?;
        let key = key.trim();
        let raw = raw.trim();
        if key.is_empty() || raw.is_empty() {
            return Err(anyhow::anyhow!(
                "{label} must use non-empty key=value syntax (got `{entry}`)"
            ));
        }
        options.insert(key.to_string(), formatter_option_value(raw));
    }
    if options.is_empty() {
        return Err(anyhow::anyhow!(
            "at least one --formatter-option key=value pair is required"
        ));
    }
    Ok(options)
}

fn formatter_option_value(raw: &str) -> serde_json::Value {
    if raw.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if raw.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(parsed) = raw.parse::<i64>() {
        return serde_json::json!(parsed);
    }
    if let Ok(parsed) = raw.parse::<f64>() {
        if parsed.is_finite() {
            return serde_json::json!(parsed);
        }
    }
    serde_json::Value::String(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        build_rule_enforcement, formatter_option_value, parse_formatter_options, parse_since_days,
        parse_validator_options, worklist_sort_key, RuleEnforcementInput,
    };
    use serde_json::json;

    #[test]
    fn parse_since_days_accepts_duration_suffix() {
        assert_eq!(parse_since_days("90d").unwrap(), 90);
        assert_eq!(parse_since_days("7").unwrap(), 7);
    }

    #[test]
    fn parse_since_days_rejects_bad_values() {
        assert!(parse_since_days("ten days").is_err());
    }

    #[test]
    fn worklist_sort_key_prefers_stable_fields() {
        let entry = json!({
            "priority_bucket": "ready_now",
            "lang": "python",
            "dep": "fastapi",
            "id": "fastapi.async-routes"
        });
        let key = worklist_sort_key(&entry);
        assert_eq!(key.0, "ready_now");
        assert_eq!(key.1, "python");
        assert_eq!(key.2, "fastapi");
        assert_eq!(key.3, "fastapi.async-routes");
    }

    #[test]
    fn worklist_sort_key_falls_back_to_secondary_fields() {
        let entry = json!({
            "priority_bucket": "later",
            "language": "rust",
            "name": "serde",
            "rule_id": "serde.deprecated-api"
        });
        let key = worklist_sort_key(&entry);
        assert_eq!(key.1, "rust");
        assert_eq!(key.2, "serde");
        assert_eq!(key.3, "serde.deprecated-api");
    }

    #[test]
    fn formatter_option_parser_coerces_bool_and_number() {
        assert_eq!(formatter_option_value("true"), json!(true));
        assert_eq!(formatter_option_value("80"), json!(80));
        assert_eq!(formatter_option_value("single"), json!("single"));
    }

    #[test]
    fn parse_formatter_options_requires_key_value_pairs() {
        assert!(parse_formatter_options(&["quote-style=single".into()]).is_ok());
        assert!(parse_formatter_options(&["broken".into()]).is_err());
    }

    #[test]
    fn parse_validator_options_requires_key_value_pairs() {
        assert!(parse_validator_options(&["command=python3 scripts/check.py".into()]).is_ok());
        assert!(parse_validator_options(&["broken".into()]).is_err());
    }

    #[test]
    fn rule_enforcement_builder_rejects_multiple_modes() {
        let err = build_rule_enforcement(RuleEnforcementInput {
            match_regex: Some("foo".into()),
            lint_tool: Some("ruff".into()),
            lint_code: Some("B006".into()),
            formatter_tool: None,
            formatter_options: Vec::new(),
            test_runner: None,
            test_path: None,
            test_selector: None,
            validator_adapter: None,
            validator_rule: None,
            validator_configs: Vec::new(),
            advisory: false,
        })
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("choose exactly one enforcement mode"));
    }

    #[test]
    fn rule_enforcement_builder_accepts_validator_mode() {
        let enforcement = build_rule_enforcement(RuleEnforcementInput {
            match_regex: None,
            lint_tool: None,
            lint_code: None,
            formatter_tool: None,
            formatter_options: Vec::new(),
            test_runner: None,
            test_path: None,
            test_selector: None,
            validator_adapter: Some("command".into()),
            validator_rule: Some("custom.inline-handlers".into()),
            validator_configs: vec!["path=scripts/check-inline-handlers.py".into()],
            advisory: false,
        })
        .unwrap();
        match enforcement {
            crate::rule_authoring::EnforcementMode::Validator {
                adapter,
                rule,
                config,
            } => {
                assert_eq!(adapter, "command");
                assert_eq!(rule, "custom.inline-handlers");
                assert_eq!(
                    config.get("path"),
                    Some(&json!("scripts/check-inline-handlers.py"))
                );
            }
            _ => panic!("expected validator enforcement mode"),
        }
    }
}
