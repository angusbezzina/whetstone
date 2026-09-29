//! Six public workflows backed by one typed service boundary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use crate::domain::{AttestationVerdict, FlagVerdict, HandTrigger, Reviewer};
use crate::service::{
    AttestRequest, BriefRequest, ChangeDefinition, ChangeKind, ChangeRequest, CheckRequest,
    CommandService, DashRequest, FlagRequest, GateMode, HandRequest, InitAction, InitRequest,
    ReviewRequest, ServiceRequest, ServiceResponse, ServiceState,
};
use crate::storage::ProjectLayout;
use crate::{check, dashboard, dashboard_service, output, rules};

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

#[derive(Subcommand)]
enum Command {
    /// Inspect the project, agree the mission, principles and rules, install
    /// the tools, and wire the skill, hooks and CI.
    Init {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, value_enum, default_value_t = InitActionArg::Inspect)]
        action: InitActionArg,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long = "resume")]
        resume_token: Option<String>,
        /// What the project exists to do, in one sentence (agree).
        #[arg(long)]
        mission: Option<String>,
        /// A pstack principle id from the catalogue (agree; repeatable).
        #[arg(long = "principle")]
        principles: Vec<String>,
        /// A principle in your own words (agree; repeatable).
        #[arg(long = "custom-principle")]
        custom_principles: Vec<String>,
        /// A starter rule to accept, or `all` (agree; repeatable).
        #[arg(long = "starter")]
        starters: Vec<String>,
        /// Show the exact records or files that would be written, and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// List the steps and the exact commands in the terminal instead of
        /// opening the dashboard's guided onboarding (inspect).
        #[arg(long)]
        no_open: bool,
        /// Agent host: claude, cursor, codex or agents (wire, setup; repeatable).
        #[arg(long = "host")]
        hosts: Vec<String>,
        /// Replace the team-owned driver script with a fresh scaffold (wire).
        #[arg(long)]
        regenerate_driver: bool,
        /// A directory to read: a verify skill (import) or an exemplar codebase (exemplar).
        #[arg(long = "from")]
        import_from: Option<PathBuf>,
        /// A Git URL of an exemplar codebase, cloned shallowly with hooks off (exemplar).
        #[arg(long = "url")]
        exemplar_url: Option<String>,
        /// Install the Git pre-commit and pre-push gates and each host's hooks (wire).
        #[arg(long)]
        hooks: bool,
        /// Scaffold the GitHub workflow that runs the required whetstone/policy check (wire).
        #[arg(long)]
        ci: bool,
        /// Run the install commands setup found (setup): the one confirmation.
        #[arg(long)]
        yes: bool,
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
        /// Run as an agent-host hook: `session-start` prints the canonical
        /// context for the host (never pointing at a stale skill).
        #[arg(long, value_enum)]
        hook: Option<DashHookArg>,
        /// The agent host the hook serves (claude, cursor, codex, agents).
        #[arg(long, default_value = "claude")]
        host: String,
    },

    /// Draft a change for the owner to accept, or record the owner's call on
    /// a draft, a flag or a raised hand.
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
        /// Typed rule, principle, feature or map details as a JSON object.
        #[arg(long, value_parser = parse_change_definition)]
        definition: Option<Box<ChangeDefinition>>,
        /// Change the accountable owner on the proposed record revision.
        #[arg(long)]
        new_owner: Option<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        expected_effect: Option<String>,
        #[arg(long)]
        impact: Option<String>,
        #[arg(long = "example")]
        examples: Vec<String>,
        #[arg(long = "conflict")]
        conflicts: Vec<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long = "resume")]
        resume_token: Option<String>,
        /// Inspect the exact before/after without recording it.
        #[arg(long, visible_alias = "dry-run")]
        preview: bool,
        /// Accept a pending draft by its proposal id (the owner's call).
        #[arg(long, conflicts_with_all = ["withdraw", "kind", "content", "definition"])]
        accept: Option<String>,
        /// Withdraw a pending draft by its proposal id.
        #[arg(long, conflicts_with_all = ["accept", "kind", "content", "definition"])]
        withdraw: Option<String>,
        /// Propose retiring an accepted record (history stays); needs --rationale.
        #[arg(long, conflicts_with_all = ["accept", "withdraw", "kind", "content", "definition"])]
        retire: Option<String>,
        /// Label a flag as right: the receipt id that raised it; needs --rationale.
        #[arg(long, conflicts_with_all = ["dismiss_flag", "accept", "withdraw", "retire", "kind"])]
        accept_flag: Option<String>,
        /// Label a flag as wrong (a false flag); needs --rationale.
        #[arg(long, conflicts_with_all = ["accept_flag", "accept", "withdraw", "retire", "kind"])]
        dismiss_flag: Option<String>,
        /// Answer a raised hand (its Beads issue id); the answer is --content.
        #[arg(long, conflicts_with_all = ["accept", "withdraw", "retire", "kind"])]
        answer: Option<String>,
        /// Draft principles from earlier values and philosophy records.
        #[arg(long, conflicts_with_all = ["accept", "withdraw", "retire", "kind", "tune"])]
        migrate: bool,
        /// Record the demotion and promotion drafts the rules' records suggest.
        #[arg(long, conflicts_with_all = ["accept", "withdraw", "retire", "kind", "migrate"])]
        tune: bool,
        /// With --kind rule: the Jev rule this mechanical rule hardens.
        #[arg(long, requires = "kind")]
        hardens: Option<String>,
    },

    /// Run the rules and return actionable findings, or record a review, a
    /// brief or a raised hand.
    Check {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long = "lang")]
        language: Option<String>,
        /// Run only this rule (repeatable); with --raise-hand, the rule the hand is about.
        #[arg(long = "rule")]
        rules: Vec<String>,
        /// Prove a mapped feature by running the rules that prove it.
        #[arg(long = "feature")]
        features: Vec<String>,
        /// Run the rules that apply to what changed and the features it touched.
        #[arg(long, conflicts_with_all = ["sweep", "staged"])]
        changed: bool,
        /// Include paths changed since this revision (e.g. origin/main), so a
        /// committed change is checked too; implies --changed.
        #[arg(long, conflicts_with = "staged")]
        base: Option<String>,
        /// Pre-commit: the fast mechanical content checks on the staged index only.
        #[arg(long, conflicts_with_all = ["sweep", "changed"])]
        staged: bool,
        /// With one --feature: a change-specific drive step, run after the
        /// feature's accepted steps and recorded in the receipt (repeatable).
        #[arg(long = "step", requires = "features")]
        steps: Vec<String>,
        /// Drive every mapped feature in sweep order.
        #[arg(long)]
        sweep: bool,
        /// Also prove each feature with its recorded mutations applied (in an
        /// isolated worktree); a proof that still passes is hollow.
        #[arg(long)]
        mutate: bool,
        /// Per-rule time bound in seconds (default 900).
        #[arg(long)]
        timeout: Option<u64>,
        /// Show which rules and exact commands would run; execute nothing.
        #[arg(long)]
        dry_run: bool,
        /// Record pstack's maintain-verification-skill outcome as a receipt.
        #[arg(long, value_enum, conflicts_with_all = ["paths", "rules", "features", "changed", "sweep", "dry_run"])]
        maintain_outcome: Option<MaintainOutcomeArg>,
        /// The maintain run's notes file (repository path) or PR URL.
        #[arg(long, requires = "maintain_outcome")]
        maintain_evidence: Option<String>,
        /// The required CI check (whetstone/policy): the team's shared, accepted
        /// rules only, never a contributor's private drafts. Use with --base.
        #[arg(long, conflicts_with_all = ["sweep", "maintain_outcome", "staged"])]
        ci: bool,
        /// Record a review verdict for this rule at the current commit.
        #[arg(long, requires = "verdict")]
        attest: Option<String>,
        /// The review verdict (with --attest).
        #[arg(long, value_enum)]
        verdict: Option<VerdictArg>,
        /// Who reviewed: interrogate (pstack's /interrogate) or a person's name.
        #[arg(long, default_value = "interrogate")]
        reviewer: String,
        /// Notes for an attestation or a brief.
        #[arg(long)]
        notes: Option<String>,
        /// Evidence for an attestation: a repository path or URL.
        #[arg(long)]
        evidence: Option<String>,
        /// Record a brief (pstack /how, /why, /blast-radius) for --area.
        #[arg(long, requires = "area")]
        brief: bool,
        /// The feature id or path the brief covers.
        #[arg(long)]
        area: Option<String>,
        /// A pstack skill the brief came from (repeatable).
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// Existing code, components or tokens the work reuses (repeatable).
        #[arg(long = "reuse")]
        reuse: Vec<String>,
        /// What the change could break (repeatable).
        #[arg(long = "risk")]
        risks: Vec<String>,
        /// Raise a hand: file a question for the owner as a `human` Beads issue.
        #[arg(long, requires_all = ["question", "tried", "recommend"])]
        raise_hand: bool,
        /// The one decision needed (with --raise-hand).
        #[arg(long)]
        question: Option<String>,
        /// What was tried (with --raise-hand).
        #[arg(long)]
        tried: Option<String>,
        /// What you recommend (with --raise-hand).
        #[arg(long)]
        recommend: Option<String>,
        /// Why you stopped (with --raise-hand).
        #[arg(long, value_enum, default_value_t = TriggerArg::VagueSpec)]
        trigger: TriggerArg,
        /// Record an explicit checkpoint for this agent host (claude, cursor, codex, agents).
        #[arg(long)]
        host: Option<String>,
        /// Run as a hook: `stop` returns violations to the same agent session
        /// (bounded); `pre-commit` and `pre-push` are the Git gates.
        #[arg(long, value_enum)]
        hook: Option<CheckHookArg>,
        /// The commit being pushed (passed by the pre-push hook).
        #[arg(long, hide = true)]
        pushed: Option<String>,
    },

    /// Receive shared records from the team's Beads remote. Nothing received
    /// is executed or accepted; local drafts stay as they are.
    Pull {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        /// Report the remote that would be pulled; receive nothing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Share an exact, confirmed package of accepted records with the team:
    /// run once to review the package, then again with --confirm <token>.
    Push {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        /// Record id to share (repeatable); default: every accepted record not yet shared.
        #[arg(long = "select")]
        select: Vec<String>,
        /// Also share this record's draft or withdrawn revisions (repeatable).
        #[arg(long = "include-ancestry")]
        include_ancestry: Vec<String>,
        /// Text that must never leave this machine; any match blocks the push.
        #[arg(long = "canary")]
        canaries: Vec<String>,
        /// The token from the reviewed package; any change invalidates it.
        #[arg(long)]
        confirm: Option<String>,
        /// Show the exact package, destination and base; share nothing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Internal schema gate retained for repository maintenance.
    #[command(hide = true)]
    Validate {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Internal golden-example gate: every rule against its labelled examples,
    /// per-rule precision and recall, and the pstack round-trip contract.
    #[command(hide = true)]
    Eval {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        lang: Option<String>,
    },

    /// Internal deterministic scanner retained for repository maintenance.
    #[command(hide = true)]
    Scan {
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long = "rule")]
        rules: Vec<String>,
        #[arg(long)]
        rules_dir: Option<PathBuf>,
        #[arg(long)]
        no_fail: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InitActionArg {
    Inspect,
    Agree,
    Cancel,
    Wire,
    Import,
    Setup,
    Exemplar,
}

impl From<InitActionArg> for InitAction {
    fn from(value: InitActionArg) -> Self {
        match value {
            InitActionArg::Inspect => Self::Inspect,
            InitActionArg::Agree => Self::Agree,
            InitActionArg::Cancel => Self::Cancel,
            InitActionArg::Wire => Self::Wire,
            InitActionArg::Import => Self::Import,
            InitActionArg::Setup => Self::Setup,
            InitActionArg::Exemplar => Self::Exemplar,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CheckHookArg {
    Stop,
    PreCommit,
    PrePush,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DashHookArg {
    SessionStart,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum MaintainOutcomeArg {
    Clean,
    Changed,
    Blocked,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum VerdictArg {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TriggerArg {
    LowConfidence,
    SecondFailedRepair,
    MustRuleArea,
    VagueSpec,
    RuleFlag,
    Unavailable,
}

impl From<TriggerArg> for HandTrigger {
    fn from(value: TriggerArg) -> Self {
        match value {
            TriggerArg::LowConfidence => Self::LowConfidence,
            TriggerArg::SecondFailedRepair => Self::SecondFailedRepair,
            TriggerArg::MustRuleArea => Self::MustRuleArea,
            TriggerArg::VagueSpec => Self::VagueSpec,
            TriggerArg::RuleFlag => Self::RuleFlag,
            TriggerArg::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ChangeKindArg {
    Mission,
    Principle,
    Rule,
    Feature,
    Map,
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
            ChangeKindArg::Feature => Self::Feature,
            ChangeKindArg::Map => Self::Map,
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
            expected_revision,
            resume_token,
            mission,
            principles,
            custom_principles,
            starters,
            dry_run,
            no_open,
            hosts,
            regenerate_driver,
            import_from,
            exemplar_url,
            hooks,
            ci,
            yes,
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
                expected_revision,
                resume_token,
                mission,
                principles,
                custom_principles,
                starters,
                dry_run,
                hosts,
                regenerate_driver,
                import_from,
                exemplar_url,
                hooks,
                ci,
                yes,
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
            hook,
            host,
        }) => {
            if matches!(hook, Some(DashHookArg::SessionStart)) {
                let input = hook_input();
                if host == "claude" && input.get("cursor_version").is_some() {
                    // Cursor runs Claude hooks too; its own hook serves it.
                    return 0;
                }
                remember_session_base(&project_dir, &input);
                let response = service.execute(ServiceRequest::Dash(DashRequest::basic(
                    project_dir,
                    request_id,
                )));
                let value = serde_json::to_value(&response).unwrap_or(json!({}));
                println!(
                    "{}",
                    crate::hosts::session_output(
                        &host,
                        &crate::hosts::session_context(&value, &host)
                    )
                );
                return 0;
            }
            if explicit_json || trail {
                let response = service.execute(ServiceRequest::Dash(DashRequest {
                    project_dir,
                    request_id,
                    search,
                    as_of,
                    page_size,
                    expected_snapshot: None,
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
            new_owner,
            rationale,
            source,
            expected_effect,
            impact,
            examples,
            conflicts,
            expected_revision,
            resume_token,
            preview,
            accept,
            withdraw,
            retire,
            accept_flag,
            dismiss_flag,
            answer,
            migrate,
            tune,
            hardens,
        }) => service.execute(ServiceRequest::Change(ChangeRequest {
            project_dir,
            request_id,
            kind: kind.map(Into::into),
            record_id,
            content,
            definition,
            new_owner,
            rationale,
            source,
            expected_effect,
            impact,
            examples,
            conflicts,
            expected_revision,
            resume_token,
            preview,
            review: accept
                .map(|proposal| ReviewRequest {
                    proposal,
                    verdict: crate::domain::LocalReviewVerdict::Accept,
                })
                .or_else(|| {
                    withdraw.map(|proposal| ReviewRequest {
                        proposal,
                        verdict: crate::domain::LocalReviewVerdict::Withdraw,
                    })
                }),
            retire,
            flag: accept_flag
                .map(|receipt| FlagRequest {
                    receipt,
                    verdict: FlagVerdict::Accept,
                })
                .or_else(|| {
                    dismiss_flag.map(|receipt| FlagRequest {
                        receipt,
                        verdict: FlagVerdict::Dismiss,
                    })
                }),
            answer,
            migrate,
            tune,
            hardens,
        })),
        Some(Command::Check {
            project_dir,
            request_id,
            paths,
            language,
            rules,
            features,
            changed,
            base,
            staged,
            steps,
            sweep,
            mutate,
            timeout,
            dry_run,
            maintain_outcome,
            maintain_evidence,
            ci,
            attest,
            verdict,
            reviewer,
            notes,
            evidence,
            brief,
            area,
            skills,
            reuse,
            risks,
            raise_hand,
            question,
            tried,
            recommend,
            trigger,
            host,
            hook,
            pushed,
        }) => {
            let hand_rule = rules.first().cloned();
            let request = CheckRequest {
                project_dir,
                request_id,
                paths,
                language,
                rules: if raise_hand { Vec::new() } else { rules },
                features,
                gate_mode: if staged || hook == Some(CheckHookArg::PreCommit) {
                    GateMode::Staged
                } else if sweep {
                    GateMode::Sweep
                } else if changed || base.is_some() || hook == Some(CheckHookArg::Stop) {
                    GateMode::Changed
                } else {
                    GateMode::All
                },
                timeout_seconds: timeout,
                dry_run,
                maintain_outcome: maintain_outcome.map(|outcome| match outcome {
                    MaintainOutcomeArg::Clean => crate::service::MaintainOutcome::Clean,
                    MaintainOutcomeArg::Changed => crate::service::MaintainOutcome::Changed,
                    MaintainOutcomeArg::Blocked => crate::service::MaintainOutcome::Blocked,
                }),
                maintain_evidence,
                ci,
                host: host
                    .clone()
                    .or_else(|| (hook == Some(CheckHookArg::Stop)).then(|| "claude".to_string())),
                base,
                pushed,
                steps,
                attest: attest.map(|rule| AttestRequest {
                    rule,
                    verdict: match verdict {
                        Some(VerdictArg::Fail) => AttestationVerdict::Fail,
                        _ => AttestationVerdict::Pass,
                    },
                    reviewer: if reviewer.trim_start_matches('/') == "interrogate" {
                        Reviewer::Interrogate
                    } else {
                        Reviewer::Person {
                            name: reviewer.clone(),
                        }
                    },
                    notes: notes.clone().unwrap_or_default(),
                    evidence,
                }),
                brief: (brief).then(|| BriefRequest {
                    area: area.clone().unwrap_or_default(),
                    skills,
                    reuse,
                    risks,
                    notes: notes.clone().unwrap_or_default(),
                }),
                mutate,
                raise_hand: raise_hand.then(|| HandRequest {
                    question: question.clone().unwrap_or_default(),
                    tried: tried.clone().unwrap_or_default(),
                    recommendation: recommend.clone().unwrap_or_default(),
                    trigger: trigger.into(),
                    rule: hand_rule,
                }),
            };
            match hook {
                Some(CheckHookArg::Stop) => {
                    return run_stop_hook(&service, request, host.as_deref().unwrap_or("claude"))
                }
                Some(CheckHookArg::PreCommit | CheckHookArg::PrePush) => {
                    return run_git_hook(&service, request, hook == Some(CheckHookArg::PreCommit))
                }
                None => service.execute(ServiceRequest::Check(request)),
            }
        }
        Some(Command::Pull {
            project_dir,
            request_id,
            dry_run,
        }) => service.execute(ServiceRequest::Pull(crate::sync::PullRequest {
            project_dir,
            request_id,
            dry_run,
        })),
        Some(Command::Push {
            project_dir,
            request_id,
            select,
            include_ancestry,
            canaries,
            confirm,
            dry_run,
        }) => service.execute(ServiceRequest::Push(crate::sync::PushRequest {
            project_dir,
            request_id,
            select,
            include_ancestry,
            canaries,
            confirm,
            dry_run,
        })),
        Some(Command::Validate { project_dir }) => return validate(&project_dir, machine),
        Some(Command::Eval { project_dir, lang }) => {
            return eval(&project_dir, lang.as_deref(), machine)
        }
        Some(Command::Scan {
            paths,
            project_dir,
            lang,
            rules,
            rules_dir,
            no_fail,
        }) => {
            return scan(
                &project_dir,
                &paths,
                lang.as_deref(),
                &rules,
                rules_dir.as_deref(),
                machine,
                no_fail,
            )
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
    let backend = Arc::new(
        dashboard_service::CommandDashboardBackend::new(project_dir, request_id).observed(
            Arc::new(move |response: &ServiceResponse| {
                if let Some(line) = progress_line(response) {
                    let _ = progress.send(line);
                }
            }),
        ),
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
         agent works under; principles from pstack and exemplar repositories are optional.\n\
         Nothing is shared, published or committed: recording it writes private state\n\
         inside this repository's Git directory, and no working-tree files.\n\n\
         The dashboard walks you through it, then stays open as your project view:\n\
         \x20 {url}\n\n\
         Prefer the terminal? Press Ctrl-C, then:\n\
         \x20 wh init --no-open            lists each step and the exact command\n\
         \x20 wh init --action agree ...   records them without the dashboard (--dry-run previews)\n\n\
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
/// that changed something. Previews, dry runs and inspections print nothing.
fn progress_line(response: &ServiceResponse) -> Option<String> {
    let flag = |key: &str| response.data.get(key) == Some(&serde_json::Value::Bool(true));
    if flag("dry_run") || flag("preview_only") || flag("read_only") {
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
        "init" => {
            let complete = response.data.pointer("/progress/agreement_complete")
                == Some(&serde_json::Value::Bool(true));
            let wrote = response
                .data
                .get("records")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|records| !records.is_empty());
            if complete && wrote {
                let revision = response.expected_revision.unwrap_or(1);
                Some(format!(
                    "Agreement recorded · revision {revision} · private, unshared. The dashboard now shows your project.\n\
                     Next: wh init --action wire --host claude --hooks (or cursor, codex, agents) writes the verification skill and the Git gates; wh check runs the first gate."
                ))
            } else if response.state == ServiceState::Success {
                Some(first_sentence(&response.summary))
            } else {
                None
            }
        }
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
        "Whetstone · init · no agreement here yet\n\nNothing is shared, published or committed: recording an agreement writes\nprivate state inside this repository's Git directory, and no working-tree files.\n\nStill needed:"
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
        "\nRecord it either way:\n\n  wh init        opens the dashboard (so does wh dash), walks you through it,\n                 shows the exact records before saving, then records them.\n\n  wh init --action agree --expected-revision {} --resume <token> \\\n    --mission \"…\" [--principle <pstack id>]... [--custom-principle \"…\"]... \\\n    --starter all\n                 records the same thing without the dashboard. Add --dry-run to\n                 see the exact records first.",
        response.expected_revision.unwrap_or(0)
    );
    if let Some(token) = &response.resume_token {
        let _ = writeln!(out, "\nToken for this inspection: {token}");
    }
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
    if let Some(revision) = response.expected_revision {
        let _ = writeln!(rendered, "Expected revision: {revision}");
    }
    if let Some(token) = &response.resume_token {
        let _ = writeln!(rendered, "Resume: {token}");
    }
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
/// expects, why, how to repair it and how to verify the repair.
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
            ("Verify", "verification_command"),
        ] {
            if let Some(value) = finding.get(field).and_then(serde_json::Value::as_str) {
                lines.push(format!("{label}: {value}"));
            }
        }
    }
    lines
}

fn validate(project_dir: &Path, machine: bool) -> i32 {
    let (report, ok) = rules::validate_schema_and_fixtures(project_dir);
    if machine {
        output::print_json(&json!({
            "status": if ok { "ok" } else { "validation_failed" },
            "ok": ok,
            "report": report,
        }));
    } else {
        print!("{report}");
    }
    i32::from(!ok)
}

/// The legacy rule-file eval (until those rules move to Beads) and the rule
/// eval: labelled examples per rule and enforcer, and the pstack round trip.
fn eval(project_dir: &Path, lang: Option<&str>, machine: bool) -> i32 {
    let mut result = check::eval(project_dir, lang);
    let rules = crate::eval::evaluate(project_dir);
    let legacy_ok = result.get("ok").and_then(|value| value.as_bool()) == Some(true);
    let rules_ok = rules["ok"] == true;
    result["legacy_ok"] = json!(legacy_ok);
    result["ok"] = json!(legacy_ok && rules_ok);
    result["status"] = json!(if legacy_ok && rules_ok {
        "ok"
    } else {
        "eval_failed"
    });
    let summary = format!(
        "Rule eval: {} rule(s), {} labelled example(s) checked, {} mismatch(es), {} error(s); pstack round trip {}.\n",
        rules["rules_scored"],
        rules["examples_checked"],
        rules["mismatch_count"],
        rules["error_count"],
        if rules["pstack_roundtrip"]["ok"] == true { "ok" } else { "FAILED" }
    );
    result["rule_eval"] = rules;
    if machine {
        output::print_json(&result);
    } else {
        print!("{}", check::format_eval_output(&result));
        print!("{summary}");
        if let Some(rules) = result["rule_eval"]["rules"].as_array() {
            for rule in rules {
                println!(
                    "  {} [{} · {}] precision {} recall {} ({} checked, {} unscored)",
                    rule["rule"].as_str().unwrap_or_default(),
                    rule["strength"].as_str().unwrap_or_default(),
                    rule["enforcer"].as_str().unwrap_or_default(),
                    rule["precision"].as_str().unwrap_or("n/a"),
                    rule["recall"].as_str().unwrap_or("n/a"),
                    rule["score"]["true_flags"].as_u64().unwrap_or(0)
                        + rule["score"]["false_flags"].as_u64().unwrap_or(0)
                        + rule["score"]["true_passes"].as_u64().unwrap_or(0)
                        + rule["score"]["missed_flags"].as_u64().unwrap_or(0),
                    rule["score"]["unscored"].as_u64().unwrap_or(0),
                );
            }
        }
    }
    i32::from(!(legacy_ok && rules_ok))
}

fn scan(
    project_dir: &Path,
    paths: &[PathBuf],
    lang: Option<&str>,
    rule_filter: &[String],
    rules_dir: Option<&Path>,
    machine: bool,
    no_fail: bool,
) -> i32 {
    let scan_paths: Vec<PathBuf> = paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                project_dir.join(path)
            }
        })
        .collect();
    let filter = (!rule_filter.is_empty()).then_some(rule_filter);
    let result = check::run(check::CheckOptions {
        project_dir,
        rules_dir,
        scan_paths: &scan_paths,
        lang_filter: lang,
        rule_filter: filter,
        execute_command_validators: true,
    });
    let violations = result
        .get("violations_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let config_issues = result
        .get("config_issues_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if machine {
        output::print_json(&result);
    } else {
        print!("{}", check::format_human_output(&result));
    }
    i32::from(!no_fail && (violations > 0 || config_issues > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_enums_map_to_service_types() {
        assert_eq!(InitAction::from(InitActionArg::Agree), InitAction::Agree);
        assert_eq!(ChangeKind::from(ChangeKindArg::Rule), ChangeKind::Rule);
        assert_eq!(InitAction::from(InitActionArg::Setup), InitAction::Setup);
        assert_eq!(
            HandTrigger::from(TriggerArg::SecondFailedRepair),
            HandTrigger::SecondFailedRepair
        );
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
            expected_revision: Some(0),
            resume_token: Some("resume-v1:abc".into()),
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
        assert!(rendered.contains("resume-v1:abc"), "{rendered}");
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
            expected_revision: Some(1),
            resume_token: None,
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
        let agreed = envelope(
            "init",
            ServiceState::NeedsInput,
            "The private project agreement is approved and installed; setup remains incomplete.",
            json!({"progress": {"agreement_complete": true}, "records": [{"id": "mission.project"}]}),
        );
        let line = progress_line(&agreed).expect("the agreement is reported");
        assert!(
            line.starts_with("Agreement recorded · revision 1"),
            "{line}"
        );
        assert!(line.contains("Next: wh init --action wire"), "{line}");

        let mut dry = agreed.clone();
        dry.data["dry_run"] = json!(true);
        assert_eq!(progress_line(&dry), None, "dry runs print nothing");
        let inspected = envelope(
            "init",
            ServiceState::NeedsInput,
            "Inspection is complete and read-only.",
            json!({"read_only": true, "progress": {"missing_decisions": ["mission"]}}),
        );
        assert_eq!(progress_line(&inspected), None, "inspections print nothing");

        let draft = envelope(
            "change",
            ServiceState::Success,
            "The private draft was recorded. It is not in force until you accept it; nothing was shared.",
            json!({"recorded": true, "record": {"id": "guidance.review"}}),
        );
        assert_eq!(
            progress_line(&draft).as_deref(),
            Some("guidance.review: The private draft was recorded")
        );
        let preview = envelope(
            "change",
            ServiceState::NeedsDecision,
            "Review the exact local proposal.",
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
            "project:abc:check: Required evidence is missing. (unknown)\nUNKNOWN whetstone.native-scan: …",
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
            expected_revision: None,
            resume_token: None,
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
                "verification_command": "wh check --json"
            }]}]}}),
        };

        let rendered = format_human_response(&response);
        for expected in [
            "Next: repair, then wh check --rule team.lowercase-functions",
            "Finding: src/app.py:7 [team.lowercase-functions]",
            "Expected: Use a lowercase function name.",
            "Repair: Rename the function.",
            "Verify: wh check --json",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }
    }
}
