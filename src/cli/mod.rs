//! Four public workflows backed by one typed service boundary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use crate::service::{
    ChangeDefinition, ChangeKind, ChangeRequest, CheckRequest, CommandService, DashRequest,
    GateMode, InitAction, InitRequest, ServiceRequest, ServiceResponse, ServiceState,
};
use crate::{dashboard, dashboard_service, output};

mod hooks;

use hooks::*;

#[derive(Parser)]
#[command(
    name = "whetstone",
    version,
    about = "Agreed rules that every agent and person is briefed on, checked against and told when to ask",
    disable_help_subcommand = true
)]
struct Cli {
    /// Emit the stable whetstone.command-response.v1 JSON envelope.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

// Parsed once per process, so the size of its largest variant does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    /// Inspect the project, draft the mission, principles and rules, and
    /// wire the hooks and CI.
    Init {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, value_enum, default_value_t = InitActionArg::Inspect)]
        action: InitActionArg,
        #[arg(long)]
        request_id: Option<String>,
        /// What the project exists to do, in one sentence (agree).
        #[arg(long)]
        mission: Option<String>,
        /// A principle id from the catalogue (agree; repeatable).
        #[arg(long = "principle")]
        principles: Vec<String>,
        /// A principle in your own words (agree; repeatable).
        #[arg(long = "custom-principle")]
        custom_principles: Vec<String>,
        /// A starter rule to accept, or `all` (agree; repeatable).
        #[arg(long = "starter")]
        starters: Vec<String>,
        /// Show the exact rulebook entries or files that would be written, and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// List the steps and the exact commands in the terminal instead of
        /// opening the dashboard's guided onboarding (inspect).
        #[arg(long)]
        no_open: bool,
        /// Install the Git pre-commit and pre-push gates (wire).
        #[arg(long)]
        hooks: bool,
        /// Scaffold the GitHub workflow that runs the required whetstone/policy check (wire).
        #[arg(long)]
        ci: bool,
    },

    /// Open the inspectable local dashboard.
    Dash {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        /// Filter the durable decision history without changing active context.
        #[arg(long)]
        search: Option<String>,
        /// Inspect current context and history at an RFC 3339 UTC instant.
        #[arg(long)]
        as_of: Option<String>,
        /// Bound the decision-history page returned by the shared service.
        #[arg(long, default_value_t = 100, value_parser = parse_page_size)]
        page_size: usize,
        /// Keep the dashboard read-only, including for the launched session.
        #[arg(long)]
        read_only: bool,
        /// Serve in the foreground without trying to open a browser.
        #[arg(long)]
        no_open: bool,
        /// For automation: write the one-time edit URL to this new file
        /// (mode 0600) instead of opening a browser. The capability never
        /// appears on stdout or in logs.
        #[arg(long, conflicts_with = "read_only")]
        bootstrap_file: Option<PathBuf>,
        /// Print the decision trail as show-me-your-work TSV (ts, phase,
        /// decision, why, evidence, result) instead of serving the dashboard.
        #[arg(long)]
        trail: bool,
    },

    /// Draft a change to whetstone/rulebook.json, retire a rule, or accept
    /// (commit) or withdraw the drafted edits.
    Change {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<ChangeKindArg>,
        #[arg(long)]
        record_id: Option<String>,
        #[arg(long)]
        content: Option<String>,
        /// Typed rule or principle details as a JSON object.
        #[arg(long, value_parser = parse_change_definition)]
        definition: Option<Box<ChangeDefinition>>,
        /// The rule's or principle's rationale, why a record is retired, or
        /// the body of the commit that accepts the drafted edits.
        #[arg(long)]
        rationale: Option<String>,
        /// Show the exact effect without writing or committing anything.
        #[arg(long, visible_alias = "dry-run")]
        preview: bool,
        /// Accept the drafted edits: commit whetstone/rulebook.json, and only it.
        #[arg(long, conflicts_with_all = ["withdraw", "retire", "kind", "content", "definition"])]
        accept: bool,
        /// Withdraw the drafted edits: restore whetstone/rulebook.json to its last commit.
        #[arg(long, conflicts_with_all = ["accept", "retire", "kind", "content", "definition"])]
        withdraw: bool,
        /// Retire a rule or principle: take it out of the rulebook and record
        /// why in its retired list; needs --rationale.
        #[arg(long, conflicts_with_all = ["accept", "withdraw", "kind", "content", "definition"])]
        retire: Option<String>,
    },

    /// Run the rules and return actionable findings.
    Check {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        /// Run only this rule (repeatable).
        #[arg(long = "rule")]
        rules: Vec<String>,
        /// Run the rules that apply to what changed.
        #[arg(long, conflicts_with_all = ["sweep", "staged"])]
        changed: bool,
        /// Include paths changed since this revision (e.g. origin/main), so a
        /// committed change is checked too; implies --changed.
        #[arg(long, conflicts_with = "staged")]
        base: Option<String>,
        /// Pre-commit: the fast mechanical content checks on the staged index only.
        #[arg(long, conflicts_with_all = ["sweep", "changed"])]
        staged: bool,
        /// Run only the rules below must and print one finding per flag
        /// {fingerprint, subjectDigest, class, priority, title, responsibility,
        /// unsure?}, a JSON array one per line, for a sweep's controller
        /// (findings.json).
        #[arg(long)]
        sweep: bool,
        /// With --sweep: `findings` (the default) or `json` for the envelope.
        #[arg(long, value_enum, requires = "sweep")]
        format: Option<CheckFormatArg>,
        /// Per-rule time bound in seconds (default 900).
        #[arg(long)]
        timeout: Option<u64>,
        /// Show which rules and exact commands would run; execute nothing.
        #[arg(long)]
        dry_run: bool,
        /// The required CI check (whetstone/policy): the committed rulebook
        /// only, never an uncommitted edit to it. Use with --base.
        #[arg(long, conflicts_with_all = ["sweep", "staged"])]
        ci: bool,
        /// Run as a Git gate: `pre-commit` or `pre-push`.
        #[arg(long, value_enum)]
        hook: Option<CheckHookArg>,
        /// The commit being pushed (passed by the pre-push hook).
        #[arg(long, hide = true)]
        pushed: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InitActionArg {
    Inspect,
    Agree,
    Cancel,
    Wire,
    Setup,
}

impl From<InitActionArg> for InitAction {
    fn from(value: InitActionArg) -> Self {
        match value {
            InitActionArg::Inspect => Self::Inspect,
            InitActionArg::Agree => Self::Agree,
            InitActionArg::Cancel => Self::Cancel,
            InitActionArg::Wire => Self::Wire,
            InitActionArg::Setup => Self::Setup,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CheckHookArg {
    PreCommit,
    PrePush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CheckFormatArg {
    Findings,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ChangeKindArg {
    Mission,
    Principle,
    Rule,
}

fn parse_change_definition(value: &str) -> Result<Box<ChangeDefinition>, String> {
    serde_json::from_str(value)
        .map(Box::new)
        .map_err(|error| format!("invalid change definition: {error}"))
}

fn parse_page_size(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "page size must be an integer from 1 through 200".to_string())?;
    if (1..=200).contains(&parsed) {
        Ok(parsed)
    } else {
        Err("page size must be an integer from 1 through 200".into())
    }
}

impl From<ChangeKindArg> for ChangeKind {
    fn from(value: ChangeKindArg) -> Self {
        match value {
            ChangeKindArg::Mission => Self::Mission,
            ChangeKindArg::Principle => Self::Principle,
            ChangeKindArg::Rule => Self::Rule,
        }
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let explicit_json = cli.json;
    let machine = cli.json || output::is_piped();
    let service = CommandService;
    let response = match cli.command {
        None => service.execute(ServiceRequest::Orientation),
        Some(Command::Init {
            project_dir,
            action,
            request_id,
            mission,
            principles,
            custom_principles,
            starters,
            dry_run,
            no_open,
            hooks,
            ci,
        }) => {
            // A person inspecting a project with no agreement is walked through
            // it in the dashboard. Machine callers always get the envelope.
            let guided =
                !machine && !no_open && !dry_run && matches!(action, InitActionArg::Inspect);
            let dashboard_dir = project_dir.clone();
            let dashboard_request_id = request_id.clone();
            let response = service.execute(ServiceRequest::Init(InitRequest {
                project_dir,
                request_id,
                action: action.into(),
                mission,
                principles,
                custom_principles,
                starters,
                dry_run,
                hooks,
                ci,
            }));
            if guided && needs_onboarding(&response) {
                return run_dashboard(
                    dashboard_dir,
                    dashboard_request_id,
                    false,
                    false,
                    None,
                    Some(&response),
                );
            }
            response
        }
        Some(Command::Dash {
            project_dir,
            request_id,
            search,
            as_of,
            page_size,
            read_only,
            no_open,
            bootstrap_file,
            trail,
        }) => {
            if explicit_json || trail {
                let response = service.execute(ServiceRequest::Dash(DashRequest {
                    project_dir,
                    request_id,
                    search,
                    as_of,
                    page_size,
                    trail,
                }));
                if trail && !explicit_json {
                    match response
                        .data
                        .get("trail")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some(tsv) => print!("{tsv}"),
                        None => eprintln!("{}", response.summary),
                    }
                    return response.state.exit_code();
                }
                response
            } else {
                return run_dashboard(
                    project_dir,
                    request_id,
                    read_only,
                    no_open,
                    bootstrap_file,
                    None,
                );
            }
        }
        Some(Command::Change {
            project_dir,
            request_id,
            kind,
            record_id,
            content,
            definition,
            rationale,
            preview,
            accept,
            withdraw,
            retire,
        }) => service.execute(ServiceRequest::Change(ChangeRequest {
            project_dir,
            request_id,
            kind: kind.map(Into::into),
            record_id,
            content,
            definition,
            rationale,
            preview,
            accept,
            withdraw,
            retire,
            actor: crate::service::detect_actor(),
        })),
        Some(Command::Check {
            project_dir,
            request_id,
            rules,
            changed,
            base,
            staged,
            sweep,
            format,
            timeout,
            dry_run,
            ci,
            hook,
            pushed,
        }) => {
            let request = CheckRequest {
                project_dir,
                request_id,
                rules,
                gate_mode: if staged || hook == Some(CheckHookArg::PreCommit) {
                    GateMode::Staged
                } else if sweep {
                    GateMode::Sweep
                } else if changed || base.is_some() {
                    GateMode::Changed
                } else {
                    GateMode::All
                },
                timeout_seconds: timeout,
                dry_run,
                ci,
                base,
                pushed,
            };
            let findings = sweep && format != Some(CheckFormatArg::Json);
            match hook {
                Some(CheckHookArg::PreCommit | CheckHookArg::PrePush) => {
                    return run_git_hook(&service, request, hook == Some(CheckHookArg::PreCommit))
                }
                None if findings => {
                    // A sweep prints only its findings (one JSON object per
                    // line), unless the envelope was asked for.
                    let response = service.execute(ServiceRequest::Check(request));
                    if explicit_json {
                        response
                    } else {
                        match response.data["findings_json"].as_str() {
                            Some(lines) => {
                                print!("{lines}");
                                return 0;
                            }
                            None => {
                                eprintln!("{}", response.summary);
                                return response.state.exit_code();
                            }
                        }
                    }
                }
                None if sweep => {
                    // `--format json`: the envelope, wherever it is printed.
                    let response = service.execute(ServiceRequest::Check(request));
                    emit_response(&response, true);
                    return response.state.exit_code();
                }
                None => service.execute(ServiceRequest::Check(request)),
            }
        }
    };
    emit_response(&response, machine);
    response.state.exit_code()
}

/// Serve the dashboard in the foreground and narrate what it records.
///
/// With `onboarding`, the process was started by a person's `wh init` before
/// any agreement existed: the intro explains the guided route and the
/// terminal alternative, and the browser opens on the onboarding page.
fn run_dashboard(
    project_dir: PathBuf,
    request_id: Option<String>,
    read_only: bool,
    no_open: bool,
    bootstrap_file: Option<PathBuf>,
    onboarding: Option<&ServiceResponse>,
) -> i32 {
    let (progress, recorded) = mpsc::channel::<String>();
    // A dashboard started for automation never acts for a person.
    let actor = if bootstrap_file.is_some() {
        crate::domain::ActorKind::Agent
    } else {
        crate::service::detect_actor()
    };
    let backend = Arc::new(
        dashboard_service::CommandDashboardBackend::new(project_dir, request_id)
            .acting_as(actor)
            .observed(Arc::new(move |response: &ServiceResponse| {
                if let Some(line) = progress_line(response) {
                    let _ = progress.send(line);
                }
            })),
    );
    let handle = match dashboard::DashboardHandle::start(
        dashboard::DashboardMode::Local {
            allow_mutations: !read_only,
        },
        backend,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("Whetstone dashboard could not start: {error}");
            return 4;
        }
    };

    // This is the only URL printed or suitable for logs. It contains no
    // bootstrap/session capability and remains useful for headless inspection.
    if let Some(path) = bootstrap_file {
        match handle.take_bootstrap_fragment() {
            Some(bootstrap) => {
                if let Err(error) = write_private_new_file(
                    &path,
                    format!("{}#bootstrap={bootstrap}", handle.public_url()).as_bytes(),
                ) {
                    eprintln!("The bootstrap file could not be written safely: {error}");
                    return 4;
                }
            }
            None => {
                eprintln!("No edit capability is available for this dashboard.");
                return 4;
            }
        }
    }
    match onboarding {
        Some(response) => print!("{}", format_onboarding_intro(response, handle.public_url())),
        None => println!("Whetstone dashboard: {}", handle.public_url()),
    }
    if !no_open {
        let launch_url = if read_only {
            handle.public_url().to_string()
        } else if let Some(bootstrap) = handle.take_bootstrap_fragment() {
            format!("{}#bootstrap={bootstrap}", handle.public_url())
        } else {
            handle.public_url().to_string()
        };
        if let Err(error) = launch_browser(&launch_url) {
            eprintln!(
                "A browser could not be opened ({error}); use the credential-free URL above for inspection."
            );
        }
    }
    if onboarding.is_none() {
        println!("Serving in the foreground. Press Ctrl-C to stop.");
    }

    // The foreground process owns the listener. Normal return and unwinding
    // drop the handle; process termination closes only this process's socket.
    // Meanwhile every step the dashboard records is written here, so an
    // owner who comes back to the terminal sees where they are.
    let started = Instant::now();
    loop {
        match recorded.recv_timeout(Duration::from_secs(60)) {
            Ok(line) => {
                let stamp = elapsed_stamp(started.elapsed());
                let mut lines = line.lines();
                if let Some(first) = lines.next() {
                    println!("{stamp}  {first}");
                }
                for rest in lines {
                    println!("{:width$}  {rest}", "", width = stamp.len());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                thread::park_timeout(Duration::from_secs(60));
            }
        }
    }
}

/// `+1m40s`: time since the dashboard started, for the progress lines.
fn elapsed_stamp(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds >= 3600 {
        format!("+{}h{:02}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("+{}m{:02}s", seconds / 60, seconds % 60)
    }
}

/// A person ran `wh init` and there is no agreement yet.
fn needs_onboarding(response: &ServiceResponse) -> bool {
    response.workflow == "init"
        && response.data.get("read_only") == Some(&serde_json::Value::Bool(true))
        && response
            .data
            .pointer("/progress/missing_decisions")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|missing| !missing.is_empty())
}

/// What `wh init` prints before serving the guided onboarding.
fn format_onboarding_intro(response: &ServiceResponse, url: &str) -> String {
    let missing = missing_steps(response).join(" and ");
    format!(
        "Whetstone · init · no agreement here yet\n\n\
         Still needed: {missing}. An agreement is a one-line mission and the rules your\n\
         agent works under; principles from the catalogue are optional.\n\
         Nothing is shared, published or committed: recording it drafts\n\
         whetstone/rulebook.json in the working tree, and accepting it is one commit.\n\n\
         The dashboard walks you through it, then stays open as your project view:\n\
         \x20 {url}\n\n\
         Prefer the terminal? Press Ctrl-C, then:\n\
         \x20 wh init --no-open            lists each step and the exact command\n\
         \x20 wh init --action agree ...   drafts them without the dashboard (--dry-run previews)\n\n\
         This terminal logs each step the dashboard records. Press Ctrl-C when you are done.\n"
    )
}

fn missing_steps(response: &ServiceResponse) -> Vec<&str> {
    response
        .data
        .pointer("/progress/missing_decisions")
        .and_then(serde_json::Value::as_array)
        .map(|missing| {
            missing
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect()
        })
        .unwrap_or_default()
}

/// One terminal line (plus an optional `Next:` line) for a dashboard mutation
/// that changed something. Previews and inspections print nothing.
fn progress_line(response: &ServiceResponse) -> Option<String> {
    let flag = |key: &str| response.data.get(key) == Some(&serde_json::Value::Bool(true));
    if flag("preview_only") || flag("read_only") {
        return None;
    }
    let first_sentence = |text: &str| {
        let line = text.lines().next().unwrap_or_default().trim();
        line.split_once(". ")
            .map_or(line, |(sentence, _)| sentence)
            .trim_end_matches('.')
            .to_string()
    };
    match response.workflow.as_str() {
        "change" => {
            if response.state != ServiceState::Success {
                return None;
            }
            let record_id = response
                .data
                .pointer("/record/id")
                .or_else(|| response.data.get("record_id"))
                .and_then(serde_json::Value::as_str);
            Some(match record_id {
                Some(id) => format!("{id}: {}", first_sentence(&response.summary)),
                None => first_sentence(&response.summary),
            })
        }
        "check" => {
            let summary = response.summary.lines().next().unwrap_or_default();
            let summary = summary
                .split_once(":check: ")
                .map_or(summary, |(_, rest)| rest);
            let state = format!("{:?}", response.state).to_lowercase();
            let summary = summary
                .strip_suffix(&format!(" ({state})"))
                .unwrap_or(summary);
            let mut line = format!("Check · {state}: {summary}");
            if let Some(next) = response.permitted_actions.first() {
                let _ = write!(line, "\nNext: {next}");
            }
            Some(line)
        }
        _ => (response.state == ServiceState::Success).then(|| first_sentence(&response.summary)),
    }
}

/// Create a new file readable only by the current user; never follow or
/// replace an existing path.
fn write_private_new_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn launch_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = ProcessCommand::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = ProcessCommand::new("xdg-open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = ProcessCommand::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no platform browser launcher is configured",
    ));

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::Builder::new()
        .name("whetstone-browser-launch".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map(|_| ())
}

fn emit_response(response: &ServiceResponse, machine: bool) {
    if machine {
        let value = serde_json::to_value(response).unwrap_or_else(|error| {
            json!({
                "schema": "whetstone.command-response.v1",
                "schema_version": 1,
                "request_id": response.request_id,
                "workflow": response.workflow,
                "state": "unknown",
                "summary": format!("Response serialization failed: {error}"),
                "evidence": [],
                "blocking_questions": [],
                "permitted_actions": ["report this Whetstone defect"],
                "data": {},
            })
        });
        output::print_json(&value);
        return;
    }
    print!("{}", format_human_response(response));
}

/// One line of plain guidance per missing step, matching the dashboard's
/// onboarding.
fn step_hint(step: &str) -> &'static str {
    match step {
        "mission" => "one line: what this project exists to do",
        "rules" => {
            "at least one rule in force; starters pick the cheapest enforcer this repository supports"
        }
        _ => "",
    }
}

/// `wh init` read by a person: what is missing, what each step means, and the
/// two ways to record it. The JSON envelope is unchanged.
fn format_init_walkthrough(response: &ServiceResponse) -> Option<String> {
    let missing = missing_steps(response);
    if missing.is_empty() {
        return None;
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Whetstone · init · no agreement here yet\n\nNothing is shared, published or committed: recording an agreement drafts\nwhetstone/rulebook.json in the working tree, and accepting it is one commit.\n\nStill needed:"
    );
    for (index, step) in missing.iter().enumerate() {
        let hint = step_hint(step);
        if hint.is_empty() {
            let _ = writeln!(out, "  {}. {step}", index + 1);
        } else {
            let _ = writeln!(out, "  {}. {step} — {hint}", index + 1);
        }
    }
    let starters = response
        .data
        .pointer("/onboarding/starters")
        .and_then(serde_json::Value::as_array)
        .map(|starters| {
            starters
                .iter()
                .filter_map(|starter| {
                    let id = starter.get("id")?.as_str()?;
                    let title = starter.get("title").and_then(serde_json::Value::as_str);
                    Some(match title {
                        Some(title) => format!("    {id} — {title}"),
                        None => format!("    {id}"),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !starters.is_empty() {
        let _ = writeln!(
            out,
            "\nStarter rules for this repository:\n{}",
            starters.join("\n")
        );
    }
    let _ = writeln!(
        out,
        "\nRecord it either way:\n\n  wh init        opens the dashboard (so does wh dash), walks you through it,\n                 shows the exact entries before saving, then drafts them.\n\n  wh init --action agree \\\n    --mission \"…\" [--principle <catalogue id>]... [--custom-principle \"…\"]... \\\n    --starter all\n                 drafts the same thing without the dashboard. Add --dry-run to\n                 see the exact entries first.\n\nThen wh change --accept commits whetstone/rulebook.json."
    );
    Some(out)
}

fn format_human_response(response: &ServiceResponse) -> String {
    if response.workflow == "init"
        && response.data.get("read_only") == Some(&serde_json::Value::Bool(true))
    {
        if let Some(walkthrough) = format_init_walkthrough(response) {
            return walkthrough;
        }
    }
    let mut rendered = String::new();
    let _ = writeln!(
        rendered,
        "Whetstone · {} · {:?}",
        response.workflow, response.state
    );
    let _ = writeln!(rendered, "{}", response.summary);
    for question in &response.blocking_questions {
        let _ = writeln!(rendered, "Needs: {question}");
    }
    for action in &response.permitted_actions {
        let _ = writeln!(rendered, "Next: {action}");
    }
    if let Some(gates) = response
        .data
        .get("gates")
        .and_then(serde_json::Value::as_array)
    {
        for gate in gates {
            let text = |field: &str| {
                gate.get(field)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let _ = writeln!(
                rendered,
                "Gate {} · {} · {}",
                text("state"),
                text("id"),
                text("summary")
            );
            for failure in gate
                .get("failures")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .take(10)
            {
                let _ = writeln!(
                    rendered,
                    "  {}: {}",
                    failure
                        .get("location")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("?"),
                    failure
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                );
            }
        }
        if let Some(root) = response
            .data
            .get("evidence_root")
            .and_then(serde_json::Value::as_str)
        {
            if let Some(run) = response
                .data
                .get("run_id")
                .and_then(serde_json::Value::as_str)
            {
                let _ = writeln!(rendered, "Evidence: {root}/{run}");
            }
        }
    }
    for detail in finding_detail_lines(&response.data) {
        let _ = writeln!(rendered, "{detail}");
    }
    rendered
}

/// Each finding in a check report, with what was observed, what the rule
/// expects, why, how to repair it and how to recheck the repair.
fn finding_detail_lines(data: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(results) = data
        .pointer("/report/results")
        .and_then(serde_json::Value::as_array)
    else {
        return lines;
    };
    for finding in results
        .iter()
        .filter_map(|result| result.get("findings").and_then(serde_json::Value::as_array))
        .flatten()
        .take(20)
    {
        let file = finding
            .get("file")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let line = finding
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let rule = finding
            .get("rule_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown-rule");
        lines.push(format!("Finding: {file}:{line} [{rule}]"));
        for (label, field) in [
            ("Observed", "observed"),
            ("Expected", "expected"),
            ("Why", "rationale"),
            ("Repair", "repair_direction"),
            ("Recheck", "recheck_command"),
        ] {
            if let Some(value) = finding.get(field).and_then(serde_json::Value::as_str) {
                lines.push(format!("{label}: {value}"));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_enums_map_to_service_types() {
        assert_eq!(InitAction::from(InitActionArg::Agree), InitAction::Agree);
        assert_eq!(ChangeKind::from(ChangeKindArg::Rule), ChangeKind::Rule);
        assert_eq!(InitAction::from(InitActionArg::Setup), InitAction::Setup);
    }

    #[test]
    fn human_init_output_walks_the_owner_through_the_missing_steps() {
        let response = ServiceResponse {
            schema: crate::service::RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id: "init-walkthrough".into(),
            workflow: "init".into(),
            state: ServiceState::NeedsInput,
            summary: "Inspection is complete and read-only.".into(),
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: vec!["Provide mission.".into()],
            permitted_actions: vec!["wh init --action agree".into()],
            data: json!({
                "read_only": true,
                "progress": {"missing_decisions": ["mission", "rules"]},
                "onboarding": {"starters": [{"id": "rule.prove-it-works", "title": "Prove it works"}]},
            }),
        };
        let rendered = format_human_response(&response);
        assert!(rendered.contains("no agreement here yet"), "{rendered}");
        assert!(
            rendered.contains("1. mission — one line: what this project exists to do"),
            "each step is explained: {rendered}"
        );
        assert!(
            rendered.contains("2. rules — at least one rule"),
            "{rendered}"
        );
        assert!(
            rendered.contains("rule.prove-it-works — Prove it works"),
            "the starters are offered: {rendered}"
        );
        assert!(
            !rendered.contains("--desired-outcome") && !rendered.contains("--owner"),
            "the removed decisions are gone: {rendered}"
        );
        assert!(
            rendered.contains("wh dash") && rendered.contains("--action agree"),
            "both routes are offered: {rendered}"
        );
        assert!(
            rendered.contains("Nothing is shared, published or"),
            "the footprint is stated: {rendered}"
        );
        assert!(rendered.contains("wh change --accept"), "{rendered}");
        assert!(!rendered.contains("Needs: Provide mission."), "{rendered}");
    }

    fn envelope(
        workflow: &str,
        state: ServiceState,
        summary: &str,
        data: serde_json::Value,
    ) -> ServiceResponse {
        ServiceResponse {
            schema: crate::service::RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id: "progress".into(),
            workflow: workflow.into(),
            state,
            summary: summary.into(),
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: Vec::new(),
            permitted_actions: vec!["wh check".into()],
            data,
        }
    }

    #[test]
    fn a_person_without_an_agreement_is_taken_to_the_guided_onboarding() {
        let fresh = envelope(
            "init",
            ServiceState::NeedsInput,
            "Inspection is complete and read-only.",
            json!({"read_only": true, "progress": {"missing_decisions": ["mission"]}}),
        );
        assert!(needs_onboarding(&fresh));
        let intro = format_onboarding_intro(&fresh, "http://127.0.0.1:4242");
        assert!(intro.contains("http://127.0.0.1:4242"), "{intro}");
        assert!(
            intro.contains("wh init --no-open") && intro.contains("--action agree"),
            "the terminal route stays available: {intro}"
        );
        assert!(intro.contains("Nothing is shared, published or"), "{intro}");
        assert!(intro.contains("Ctrl-C"), "{intro}");

        let established = envelope(
            "init",
            ServiceState::NeedsInput,
            "The agreement is installed locally.",
            json!({"read_only": true, "progress": {"missing_decisions": []}}),
        );
        assert!(!needs_onboarding(&established));
        let mut other = fresh.clone();
        other.workflow = "dash".into();
        assert!(!needs_onboarding(&other));
    }

    #[test]
    fn progress_lines_report_what_the_dashboard_recorded_and_nothing_else() {
        let draft = envelope(
            "change",
            ServiceState::Success,
            "whetstone/rulebook.json now holds the change to rule.review. It is a draft until you accept it, which commits the file.",
            json!({"record_id": "rule.review", "written": true}),
        );
        assert_eq!(
            progress_line(&draft).as_deref(),
            Some("rule.review: whetstone/rulebook.json now holds the change to rule.review")
        );
        let preview = envelope(
            "change",
            ServiceState::NeedsDecision,
            "Review the exact change to rule.review before it is written.",
            json!({"preview_only": true}),
        );
        assert_eq!(progress_line(&preview), None, "previews print nothing");
        let probe = envelope(
            "change",
            ServiceState::NeedsInput,
            "More owner input is required; no agreement change was recorded.",
            json!({"effects": {}}),
        );
        assert_eq!(progress_line(&probe), None, "probes print nothing");

        let check = envelope(
            "check",
            ServiceState::Unknown,
            "project:abc:check: Required evidence is missing. (unknown)\nUNKNOWN gate:rule.tests: …",
            json!({}),
        );
        assert_eq!(
            progress_line(&check).as_deref(),
            Some("Check · unknown: Required evidence is missing.\nNext: wh check")
        );

        assert_eq!(elapsed_stamp(Duration::from_secs(100)), "+1m40s");
        assert_eq!(elapsed_stamp(Duration::from_secs(3_725)), "+1h02m");
    }

    #[test]
    fn human_check_output_keeps_each_finding_actionable() {
        let response = ServiceResponse {
            schema: crate::service::RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id: "human-parity".into(),
            workflow: "check".into(),
            state: ServiceState::Violated,
            summary: "One rule failed.".into(),
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: Vec::new(),
            permitted_actions: vec!["repair, then wh check --rule team.lowercase-functions".into()],
            data: json!({"report": {"results": [{"findings": [{
                "file": "src/app.py",
                "line": 7,
                "rule_id": "team.lowercase-functions",
                "observed": "ReadConfig",
                "expected": "Use a lowercase function name.",
                "rationale": "The accepted convention requires it.",
                "repair_direction": "Rename the function.",
                "recheck_command": "wh check --json"
            }]}]}}),
        };

        let rendered = format_human_response(&response);
        for expected in [
            "Next: repair, then wh check --rule team.lowercase-functions",
            "Finding: src/app.py:7 [team.lowercase-functions]",
            "Expected: Use a lowercase function name.",
            "Repair: Rename the function.",
            "Recheck: wh check --json",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }
    }
}
