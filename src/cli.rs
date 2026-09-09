//! Six public workflows backed by one typed service boundary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use crate::domain::RepairSessionStateRecord;
use crate::repair_host::{
    BeginRepairRequest, CheckpointRequest, FinalizeRepairRequest, HostCheckpointKind,
    RepairAuthorityEvidence, RepairCompletionEvidence, RepairFeedback, RepairHost, RepairHostError,
};
#[cfg(unix)]
use crate::repair_transport::HostSocketAuthorityVerifier;
use crate::service::{
    BasicRequest, ChangeDefinition, ChangeKind, ChangeRequest, CheckRequest, CommandService,
    DashRequest, InitAction, InitRequest, ServiceRequest, ServiceResponse, ServiceState,
};
use crate::storage::ProjectLayout;
use crate::{check, dashboard, dashboard_service, output, rules};

#[derive(Parser)]
#[command(
    name = "whetstone",
    version,
    about = "Project agreements that humans and agents can trust",
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
    /// Inspect or establish the local project agreement.
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
        #[arg(long)]
        mission: Option<String>,
        #[arg(long)]
        desired_outcome: Option<String>,
        #[arg(long)]
        values: Option<String>,
        #[arg(long)]
        philosophy: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        initial_safeguard: Option<String>,
        #[arg(long)]
        safeguard_scope: Option<String>,
        #[arg(long)]
        revision_triggers: Option<String>,
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
    },

    /// Propose or record a bounded local agreement change.
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
        /// Typed metric or standard details as a JSON object.
        #[arg(long, value_parser = parse_change_definition)]
        definition: Option<Box<ChangeDefinition>>,
        /// Replace the first mission outcome while revising a mission record.
        #[arg(long)]
        desired_outcome: Option<String>,
        /// Replace the first review trigger while revising an implementation philosophy.
        #[arg(long)]
        review_triggers: Option<String>,
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
        /// Inspect the exact before/after proposal without recording it.
        #[arg(long)]
        preview: bool,
    },

    /// Evaluate applicable deterministic rules without repairing or publishing.
    Check {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long = "lang")]
        language: Option<String>,
        #[arg(long = "rule")]
        rules: Vec<String>,
        /// Checkpoint an existing host-authorized repair session.
        #[arg(long, conflicts_with_all = ["paths", "language", "rules"])]
        repair_session: Option<String>,
        /// Exact durable repair-session revision expected by the caller.
        #[arg(long, requires = "repair_session")]
        repair_revision: Option<u64>,
        /// Opaque locator interpreted only by the trusted host adapter.
        #[arg(long, requires = "repair_session")]
        authority_evidence: Option<String>,
        /// Begin a durable repair session from host-injected, authenticated task context.
        #[arg(
            long,
            requires = "repair_session",
            conflicts_with_all = ["repair_revision", "post_edit", "finalize_with"]
        )]
        begin_repair: bool,
        /// Identify this call as a supported host's post-edit callback.
        #[arg(long, requires = "repair_session", conflicts_with = "finalize_with")]
        post_edit: bool,
        /// Run broader final verification with opaque host acceptance evidence.
        #[arg(long, requires = "repair_session")]
        finalize_with: Option<String>,
    },

    /// Receive approved team agreement changes when team sync is configured.
    Pull {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
    },

    /// Submit selected local proposals for team review when sync is configured.
    Push {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
    },

    /// Internal schema gate retained for repository maintenance.
    #[command(hide = true)]
    Validate {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Internal golden-example gate retained for repository maintenance.
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
}

impl From<InitActionArg> for InitAction {
    fn from(value: InitActionArg) -> Self {
        match value {
            InitActionArg::Inspect => Self::Inspect,
            InitActionArg::Agree => Self::Agree,
            InitActionArg::Cancel => Self::Cancel,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ChangeKindArg {
    Mission,
    Value,
    Philosophy,
    Metric,
    Guidance,
    Standard,
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
            ChangeKindArg::Value => Self::Value,
            ChangeKindArg::Philosophy => Self::Philosophy,
            ChangeKindArg::Metric => Self::Metric,
            ChangeKindArg::Guidance => Self::Guidance,
            ChangeKindArg::Standard => Self::Standard,
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
            desired_outcome,
            values,
            philosophy,
            owner,
            initial_safeguard,
            safeguard_scope,
            revision_triggers,
        }) => service.execute(ServiceRequest::Init(InitRequest {
            project_dir,
            request_id,
            action: action.into(),
            expected_revision,
            resume_token,
            mission,
            desired_outcome,
            values,
            philosophy,
            owner,
            initial_safeguard,
            safeguard_scope,
            revision_triggers,
        })),
        Some(Command::Dash {
            project_dir,
            request_id,
            search,
            as_of,
            page_size,
            read_only,
            no_open,
        }) => {
            if explicit_json {
                service.execute(ServiceRequest::Dash(DashRequest {
                    project_dir,
                    request_id,
                    search,
                    as_of,
                    history_after: None,
                    page_size,
                    expected_snapshot: None,
                }))
            } else {
                return run_dashboard(project_dir, request_id, read_only, no_open);
            }
        }
        Some(Command::Change {
            project_dir,
            request_id,
            kind,
            record_id,
            content,
            definition,
            desired_outcome,
            review_triggers,
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
        }) => service.execute(ServiceRequest::Change(ChangeRequest {
            project_dir,
            request_id,
            kind: kind.map(Into::into),
            record_id,
            content,
            definition,
            desired_outcome,
            review_triggers,
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
        })),
        Some(Command::Check {
            project_dir,
            request_id,
            paths,
            language,
            rules,
            repair_session,
            repair_revision,
            authority_evidence,
            begin_repair,
            post_edit,
            finalize_with,
        }) => {
            if let Some(session_id) = repair_session {
                return run_repair_checkpoint(
                    project_dir,
                    request_id,
                    session_id,
                    repair_revision,
                    authority_evidence,
                    begin_repair,
                    post_edit,
                    finalize_with,
                    machine,
                );
            }
            service.execute(ServiceRequest::Check(CheckRequest {
                project_dir,
                request_id,
                paths,
                language,
                rules,
            }))
        }
        Some(Command::Pull {
            project_dir,
            request_id,
        }) => service.execute(ServiceRequest::Pull(BasicRequest {
            project_dir,
            request_id,
        })),
        Some(Command::Push {
            project_dir,
            request_id,
        }) => service.execute(ServiceRequest::Push(BasicRequest {
            project_dir,
            request_id,
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

#[allow(clippy::too_many_arguments)]
fn run_repair_checkpoint(
    project_dir: PathBuf,
    request_id: Option<String>,
    session_id: String,
    repair_revision: Option<u64>,
    authority_evidence: Option<String>,
    begin_repair: bool,
    post_edit: bool,
    finalize_with: Option<String>,
    machine: bool,
) -> i32 {
    let now_unix = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => {
            let response = repair_error_response(
                request_id.unwrap_or_else(|| "repair-clock-unavailable".into()),
                ServiceState::Unknown,
                "repair_clock_unavailable",
                "The current time could not be established, so repair authority freshness was not evaluated.",
                "Restore a trustworthy system clock before retrying.",
            );
            emit_response(&response, machine);
            return response.state.exit_code();
        }
    };
    let request_id =
        request_id.unwrap_or_else(|| format!("repair-{}-{}", std::process::id(), now_unix));
    let Some(authority_locator) = authority_evidence else {
        let response = repair_error_response(
            request_id,
            ServiceState::NeedsInput,
            "authority_evidence_required",
            "A repair checkpoint requires an opaque --authority-evidence locator; no session state was changed.",
            "Ask the accountable host to supply a task-bound authority locator; checking alone cannot grant editing.",
        );
        emit_response(&response, machine);
        return response.state.exit_code();
    };

    #[cfg(not(unix))]
    let response = repair_error_response(
        request_id,
        ServiceState::Unavailable,
        "repair_host_transport_unsupported",
        "The initial repair-host transport is unavailable on this platform; no session state was changed.",
        "Run ordinary wh check for observational feedback or use a host with a supported repair adapter.",
    );

    #[cfg(unix)]
    let resolved_project_root = ProjectLayout::resolve(&project_dir, None)
        .ok()
        .map(|layout| layout.project_root().to_path_buf());
    #[cfg(unix)]
    let response = match resolved_project_root.as_deref().and_then(|project_root| {
        HostSocketAuthorityVerifier::from_inherited_environment(project_root).ok()
    }) {
        None => repair_error_response(
            request_id,
            ServiceState::Unavailable,
            "repair_host_adapter_unavailable",
            "No trusted repair-host adapter is available; no session state was changed.",
            "Run ordinary wh check for observational feedback, or ask the host to install its authenticated adapter. Editing requires separate authority.",
        ),
        Some(verifier) => {
            let host = RepairHost::new(verifier);
            let authority_evidence = RepairAuthorityEvidence {
                locator: authority_locator.clone(),
            };
            let now = utc_timestamp(now_unix);
            let result = if begin_repair {
                match HostSocketAuthorityVerifier::launch_from_inherited_environment() {
                    Ok(launch) => host.begin(
                        &project_dir,
                        BeginRepairRequest {
                            session_id: session_id.clone(),
                            request_id: request_id.clone(),
                            task: launch.task,
                            authority_revision: launch.authority_revision,
                            authority_evidence,
                            context: launch.context,
                            now,
                        },
                    ),
                    Err(_) => {
                        let response = repair_error_response(
                            request_id,
                            ServiceState::Unavailable,
                            "repair_host_launch_unavailable",
                            "The host did not provide valid authenticated repair-launch context; no session was created.",
                            "Ask the accountable host to launch Whetstone with its task-bound repair adapter.",
                        );
                        emit_response(&response, machine);
                        return response.state.exit_code();
                    }
                }
            } else if let Some(completion_locator) = finalize_with {
                let Some(expected_revision) = repair_revision else {
                    let response = missing_repair_revision(request_id);
                    emit_response(&response, machine);
                    return response.state.exit_code();
                };
                host.finalize(
                    &project_dir,
                    FinalizeRepairRequest {
                        session_id: session_id.clone(),
                        request_id: request_id.clone(),
                        expected_revision,
                        authority_evidence,
                        completion_evidence: RepairCompletionEvidence {
                            locator: completion_locator,
                        },
                        now,
                    },
                )
            } else {
                let Some(expected_revision) = repair_revision else {
                    let response = missing_repair_revision(request_id);
                    emit_response(&response, machine);
                    return response.state.exit_code();
                };
                host.checkpoint(
                    &project_dir,
                    CheckpointRequest {
                        session_id: session_id.clone(),
                        request_id: request_id.clone(),
                        expected_revision,
                        authority_evidence,
                        now,
                        kind: if post_edit {
                            HostCheckpointKind::PostEditHook
                        } else {
                            HostCheckpointKind::ExplicitCheckpoint
                        },
                    },
                )
            };
            match result {
                Ok(feedback) => repair_feedback_response(
                    request_id,
                    feedback,
                    &session_id,
                    &authority_locator,
                ),
                Err(error) => repair_host_error_response(request_id, error),
            }
        }
    };

    emit_response(&response, machine);
    response.state.exit_code()
}

fn missing_repair_revision(request_id: String) -> ServiceResponse {
    repair_error_response(
        request_id,
        ServiceState::NeedsInput,
        "repair_revision_required",
        "A repair checkpoint requires --repair-revision; no session state was changed.",
        "Inspect the current private repair-session revision, then retry the same checkpoint.",
    )
}

fn repair_feedback_response(
    request_id: String,
    feedback: RepairFeedback,
    session_id: &str,
    authority_locator: &str,
) -> ServiceResponse {
    let state = match feedback.state {
        RepairSessionStateRecord::Ready => ServiceState::Violated,
        RepairSessionStateRecord::ReadyForFinalVerification => ServiceState::NeedsInput,
        RepairSessionStateRecord::Verified => ServiceState::Success,
        RepairSessionStateRecord::NeedsDecision | RepairSessionStateRecord::Cancelled => {
            ServiceState::NeedsDecision
        }
    };
    let state_summary = match feedback.state {
        RepairSessionStateRecord::Ready => {
            "The authenticated repair checkpoint found remaining violations; bounded source repair remains authorized."
        }
        RepairSessionStateRecord::ReadyForFinalVerification => {
            "The scoped repair checkpoint passed; broader final verification and host acceptance evidence are still required."
        }
        RepairSessionStateRecord::Verified => {
            "The exact repaired workspace passed broader final verification and authenticated task acceptance."
        }
        RepairSessionStateRecord::NeedsDecision => {
            "The bounded repair loop stopped and requires one accountable owner decision."
        }
        RepairSessionStateRecord::Cancelled => "The repair session is cancelled.",
    };
    let summary = format!("{state_summary}\n{}", feedback.check.summary);
    let expected_revision = feedback.session.revision;
    let required_snapshot = feedback.check.required_snapshot.clone();
    let evidence = feedback.check.evidence.clone();
    let mut permitted_actions = Vec::new();
    if feedback.edit_authorized {
        permitted_actions.push(
            "repair only source within the authenticated task scope, then run the same checkpoint"
                .into(),
        );
    }
    if feedback.state == RepairSessionStateRecord::ReadyForFinalVerification {
        permitted_actions.push(
            "obtain authenticated task-acceptance and required-review evidence, then rerun with --finalize-with"
                .into(),
        );
    }
    let blocking_questions = feedback
        .handoff
        .as_ref()
        .map(|handoff| vec![handoff.question.clone()])
        .unwrap_or_default();
    if let Some(handoff) = &feedback.handoff {
        permitted_actions.push(handoff.permitted_next_step.clone());
    }
    let data = serde_json::to_value(&feedback).unwrap_or_else(|_| {
        json!({
            "reason_code": "repair_feedback_serialization_failed"
        })
    });
    ServiceResponse {
        schema: crate::service::RESPONSE_SCHEMA.into(),
        schema_version: 1,
        request_id,
        workflow: "check".into(),
        state,
        summary,
        expected_revision: Some(expected_revision),
        resume_token: None,
        required_snapshot,
        evidence,
        blocking_questions,
        permitted_actions,
        data: json!({
            "repair": data,
            "host_callback": {
                "command_family": "check",
                "repair_session": session_id,
                "repair_revision": expected_revision,
                "authority_evidence": authority_locator,
                "post_edit": true,
                "requires_inherited_authority": true
            }
        }),
    }
}

fn repair_host_error_response(request_id: String, error: RepairHostError) -> ServiceResponse {
    let (state, reason_code, summary, next) = match error {
        RepairHostError::MissingAuthority => (
            ServiceState::Unavailable,
            "repair_authority_unavailable",
            "The host could not authenticate repair authority; no session transition was written.",
            "Ask the accountable host to restore or issue task-bound authority.",
        ),
        RepairHostError::AuthorityMismatch
        | RepairHostError::AuthorityExpired
        | RepairHostError::ForbiddenCapability
        | RepairHostError::DuplicateAuthoritySession => (
            ServiceState::NeedsDecision,
            "repair_authority_rejected",
            "The supplied host authority does not permit this exact repair transition.",
            "Return to the accountable owner; ordinary checking cannot expand authority.",
        ),
        RepairHostError::Stale { .. } => (
            ServiceState::Stale,
            "repair_session_stale",
            "The repair-session revision is stale; no transition was written.",
            "Inspect the current session revision before deciding whether to retry.",
        ),
        RepairHostError::SessionMissing => (
            ServiceState::Unavailable,
            "repair_session_missing",
            "The requested private repair session does not exist; nothing was changed.",
            "Ask the host to begin an authenticated repair session.",
        ),
        RepairHostError::CompletionRejected => (
            ServiceState::NeedsDecision,
            "repair_completion_rejected",
            "Task acceptance or required review was not authenticated for the exact final snapshot.",
            "Return the persisted handoff to the accountable owner.",
        ),
        RepairHostError::ProtectedPath(_)
        | RepairHostError::InvalidPath(_)
        | RepairHostError::WorkspaceChanged => (
            ServiceState::NeedsDecision,
            "repair_scope_rejected",
            "The workspace changed outside the authenticated repair boundary; no green result was recorded.",
            "Return to the accountable owner without weakening policy, checks, tests, or baselines.",
        ),
        RepairHostError::OperationInProgress => (
            ServiceState::NeedsDecision,
            "repair_operation_in_progress",
            "A durable operation claim already covers this session revision; Whetstone will not execute it twice.",
            "Inspect or cancel the claimed session; after an interrupted begin, the owner must issue a new authority revision.",
        ),
        RepairHostError::BudgetExhausted | RepairHostError::Terminal => (
            ServiceState::NeedsDecision,
            "repair_session_stopped",
            "The repair session cannot take another automated transition.",
            "Inspect the durable handoff or current terminal state.",
        ),
        RepairHostError::Storage(_)
        | RepairHostError::Domain(_)
        | RepairHostError::WrongRecordKind
        | RepairHostError::InvalidCheckResponse
        | RepairHostError::Workspace(_)
        | RepairHostError::WorkspaceTooLarge => (
            ServiceState::Unknown,
            "repair_checkpoint_unknown",
            "Whetstone could not establish a trustworthy repair checkpoint result.",
            "Resolve the reported local storage, schema, or workspace problem before retrying.",
        ),
    };
    repair_error_response(request_id, state, reason_code, summary, next)
}

fn repair_error_response(
    request_id: String,
    state: ServiceState,
    reason_code: &str,
    summary: &str,
    next: &str,
) -> ServiceResponse {
    ServiceResponse {
        schema: crate::service::RESPONSE_SCHEMA.into(),
        schema_version: 1,
        request_id,
        workflow: "check".into(),
        state,
        summary: summary.into(),
        expected_revision: None,
        resume_token: None,
        required_snapshot: None,
        evidence: Vec::new(),
        blocking_questions: Vec::new(),
        permitted_actions: vec![next.into()],
        data: json!({ "reason_code": reason_code }),
    }
}

fn utc_timestamp(unix_seconds: u64) -> String {
    let seconds_per_day = 86_400_u64;
    let days = (unix_seconds / seconds_per_day) as i64;
    let day_seconds = unix_seconds % seconds_per_day;
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn run_dashboard(
    project_dir: PathBuf,
    request_id: Option<String>,
    read_only: bool,
    no_open: bool,
) -> i32 {
    let backend = Arc::new(dashboard_service::CommandDashboardBackend::new(
        project_dir,
        request_id,
    ));
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
    println!("Whetstone dashboard: {}", handle.public_url());
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
    println!("Serving in the foreground. Press Ctrl-C to stop.");

    // The foreground process owns the listener. Normal return and unwinding
    // drop the handle; process termination closes only this process's socket.
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
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

fn format_human_response(response: &ServiceResponse) -> String {
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
    for detail in repair_detail_lines(&response.data) {
        let _ = writeln!(rendered, "{detail}");
    }
    rendered
}

fn repair_detail_lines(data: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(repair) = data.get("repair") else {
        return lines;
    };
    if let Some(results) = repair
        .pointer("/check/data/report/results")
        .and_then(serde_json::Value::as_array)
    {
        for finding in results
            .iter()
            .filter_map(|result| result.get("findings").and_then(serde_json::Value::as_array))
            .flatten()
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
    }
    let Some(handoff) = repair.get("handoff").and_then(serde_json::Value::as_object) else {
        return lines;
    };
    for (label, field) in [("Recommendation", "recommendation"), ("Impact", "impact")] {
        if let Some(value) = handoff.get(field).and_then(serde_json::Value::as_str) {
            lines.push(format!("{label}: {value}"));
        }
    }
    if let Some(alternatives) = handoff
        .get("alternatives")
        .and_then(serde_json::Value::as_array)
    {
        for alternative in alternatives.iter().filter_map(serde_json::Value::as_str) {
            lines.push(format!("Alternative: {alternative}"));
        }
    }
    if let Some(evidence) = handoff
        .get("evidence")
        .and_then(serde_json::Value::as_array)
    {
        for item in evidence {
            let system = item
                .get("system")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("evidence");
            let locator = item
                .get("locator")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!("Evidence: {system}:{locator}"));
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

fn eval(project_dir: &Path, lang: Option<&str>, machine: bool) -> i32 {
    let result = check::eval(project_dir, lang);
    let ok = result.get("ok").and_then(|value| value.as_bool()) == Some(true);
    if machine {
        output::print_json(&result);
    } else {
        print!("{}", check::format_eval_output(&result));
    }
    i32::from(!ok)
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
        assert_eq!(
            ChangeKind::from(ChangeKindArg::Guidance),
            ChangeKind::Guidance
        );
    }

    #[test]
    fn repair_receipt_clock_is_stable_utc() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951_827_696), "2000-02-29T12:34:56Z");
    }

    #[test]
    fn human_repair_output_preserves_actionable_json_evidence_and_handoff() {
        let response = ServiceResponse {
            schema: crate::service::RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id: "human-parity".into(),
            workflow: "check".into(),
            state: ServiceState::NeedsDecision,
            summary: "Repair stopped.".into(),
            expected_revision: Some(2),
            resume_token: None,
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: vec!["Which accountable change should proceed?".into()],
            permitted_actions: vec!["Record the owner decision with wh change.".into()],
            data: json!({
                "repair": {
                    "check": {"data": {"report": {"results": [{"findings": [{
                        "file": "src/app.py",
                        "line": 7,
                        "rule_id": "team.lowercase-functions",
                        "observed": "ReadConfig",
                        "expected": "Use a lowercase function name.",
                        "rationale": "The accepted convention requires it.",
                        "repair_direction": "Rename the function.",
                        "verification_command": "wh check --json"
                    }]}]}}},
                    "handoff": {
                        "recommendation": "Keep policy unchanged.",
                        "impact": "The repair remains paused.",
                        "alternatives": ["Authorize a reviewed scope change."],
                        "evidence": [{"system": "whetstone_check", "locator": "receipt@1"}]
                    }
                }
            }),
        };

        let rendered = format_human_response(&response);
        for expected in [
            "Needs: Which accountable change should proceed?",
            "Next: Record the owner decision with wh change.",
            "Finding: src/app.py:7 [team.lowercase-functions]",
            "Expected: Use a lowercase function name.",
            "Repair: Rename the function.",
            "Verify: wh check --json",
            "Recommendation: Keep policy unchanged.",
            "Alternative: Authorize a reviewed scope change.",
            "Impact: The repair remains paused.",
            "Evidence: whetstone_check:receipt@1",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }
    }
}
