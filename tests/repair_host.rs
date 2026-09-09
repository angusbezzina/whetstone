use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::TempDir;
use whetstone::domain::{
    DomainError, ExternalRef, ExternalSystem, PrincipalKind, PrincipalRef, ProvenanceAuthority,
    RecordBody, RecordId, RepairSessionStateRecord,
};
use whetstone::repair_host::{
    BeginRepairRequest, CheckpointRequest, FinalizeRepairRequest, HostCheckpointKind,
    RepairAuthorityEvidence, RepairAuthorityTarget, RepairAuthorityVerifier, RepairBudget,
    RepairCheckExecutor, RepairClock, RepairCompletionEvidence, RepairCompletionTarget, RepairHost,
    RepairHostError, RepairTaskContext, VerifiedRepairAuthority, VerifiedRepairCompletion,
};
use whetstone::service::{
    CheckRequest, CommandService, ServiceRequest, ServiceResponse, ServiceState, RESPONSE_SCHEMA,
};
use whetstone::storage::{DoltRepository, ProjectLayout, StorageError, StoreKind};

const NOW: &str = "2026-09-09T12:00:00Z";
const LATER: &str = "2026-09-09T13:00:00Z";

#[derive(Clone, Copy)]
enum VerifierMode {
    Trusted,
    Missing,
    ExpandedCapability,
    CompletionRejected,
}

#[derive(Clone, Copy)]
struct FixtureHostAdapter {
    mode: VerifierMode,
}

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new(now: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now)))
    }

    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl RepairClock for FakeClock {
    fn now_unix(&self) -> Result<u64, RepairHostError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Clone)]
enum CompletionBoundary {
    AdvanceTo(u64),
    Mutate(PathBuf),
    #[cfg(unix)]
    ChangeMode {
        path: PathBuf,
        mode: u32,
    },
}

#[derive(Clone)]
struct BoundaryAdapter {
    clock: FakeClock,
    expires_at_unix: u64,
    completion: CompletionBoundary,
}

impl RepairAuthorityVerifier for BoundaryAdapter {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        if evidence.locator != "host-grant:task-42" {
            return Err(RepairHostError::MissingAuthority);
        }
        Ok(VerifiedRepairAuthority {
            evidence_id: "boundary-authority".into(),
            principal: PrincipalRef {
                kind: PrincipalKind::LocalUser,
                stable_id: "owner-42".into(),
                display_name: None,
            },
            task: target.task.clone(),
            project: target.project.clone(),
            authority_revision: target.authority_revision,
            expires_at: LATER.into(),
            expires_at_unix: self.expires_at_unix,
            context: target.context.clone(),
            may_edit_source: true,
            may_edit_policy: false,
            may_edit_checks: false,
            may_reset_baselines: false,
            may_publish: false,
            may_merge: false,
            may_release: false,
        })
    }

    fn verify_completion(
        &self,
        _evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError> {
        match &self.completion {
            CompletionBoundary::AdvanceTo(now) => self.clock.set(*now),
            CompletionBoundary::Mutate(path) => {
                fs::write(path, "def changed_during_acceptance():\n    pass\n")
                    .expect("mutate during completion")
            }
            #[cfg(unix)]
            CompletionBoundary::ChangeMode { path, mode } => {
                use std::os::unix::fs::PermissionsExt;

                fs::set_permissions(path, fs::Permissions::from_mode(*mode))
                    .expect("change mode during completion")
            }
        }
        Ok(VerifiedRepairCompletion {
            evidence_id: "boundary-completion".into(),
            session_id: target.session_id.clone(),
            task: target.task.clone(),
            project: target.project.clone(),
            authority_revision: target.authority_revision,
            candidate_workspace: target.candidate_workspace.clone(),
            check_snapshot: target.check_snapshot.clone(),
            task_acceptance_satisfied: true,
            required_reviews_satisfied: true,
        })
    }
}

#[derive(Clone)]
struct BarrierAdapter {
    barrier: Arc<Barrier>,
    initial_calls: Arc<AtomicU64>,
}

#[derive(Clone)]
struct DelayedAuthorityAdapter {
    clock: FakeClock,
    calls: Arc<AtomicU64>,
    advance_on_call: u64,
}

#[derive(Clone)]
struct PausingAuthorityAdapter {
    calls: Arc<AtomicU64>,
    pause_on_call: u64,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

#[derive(Clone)]
struct CountingCheckExecutor(Arc<AtomicU64>);

impl RepairCheckExecutor for CountingCheckExecutor {
    fn execute_check(&self, request: CheckRequest) -> ServiceResponse {
        self.0.fetch_add(1, Ordering::SeqCst);
        CommandService.execute(ServiceRequest::Check(request))
    }
}

impl RepairAuthorityVerifier for DelayedAuthorityAdapter {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        let mut grant = FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify(evidence, target)?;
        grant.expires_at_unix = 200;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.advance_on_call {
            self.clock.set(201);
        }
        Ok(grant)
    }

    fn verify_completion(
        &self,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError> {
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify_completion(evidence, target)
    }
}

impl RepairAuthorityVerifier for PausingAuthorityAdapter {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let grant = FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify(evidence, target)?;
        if call == self.pause_on_call {
            self.entered.wait();
            self.release.wait();
        }
        Ok(grant)
    }

    fn verify_completion(
        &self,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError> {
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify_completion(evidence, target)
    }
}

impl RepairAuthorityVerifier for BarrierAdapter {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        if self.initial_calls.fetch_add(1, Ordering::SeqCst) < 2 {
            self.barrier.wait();
        }
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify(evidence, target)
    }

    fn verify_completion(
        &self,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError> {
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        }
        .verify_completion(evidence, target)
    }
}

impl RepairAuthorityVerifier for FixtureHostAdapter {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        if matches!(self.mode, VerifierMode::Missing) || evidence.locator != "host-grant:task-42" {
            return Err(RepairHostError::MissingAuthority);
        }
        Ok(VerifiedRepairAuthority {
            evidence_id: "authenticated-host-grant:task-42:r7".into(),
            principal: PrincipalRef {
                kind: PrincipalKind::LocalUser,
                stable_id: "owner-42".into(),
                display_name: None,
            },
            task: target.task.clone(),
            project: target.project.clone(),
            authority_revision: target.authority_revision,
            expires_at: LATER.into(),
            expires_at_unix: u64::MAX,
            context: target.context.clone(),
            may_edit_source: true,
            may_edit_policy: matches!(self.mode, VerifierMode::ExpandedCapability),
            may_edit_checks: false,
            may_reset_baselines: false,
            may_publish: false,
            may_merge: false,
            may_release: false,
        })
    }

    fn verify_completion(
        &self,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError> {
        if matches!(
            self.mode,
            VerifierMode::Missing | VerifierMode::CompletionRejected
        ) || evidence.locator != "host-acceptance:task-42"
        {
            return Err(RepairHostError::CompletionRejected);
        }
        Ok(VerifiedRepairCompletion {
            evidence_id: "authenticated-task-acceptance:task-42:r7".into(),
            session_id: target.session_id.clone(),
            task: target.task.clone(),
            project: target.project.clone(),
            authority_revision: target.authority_revision,
            candidate_workspace: target.candidate_workspace.clone(),
            check_snapshot: target.check_snapshot.clone(),
            task_acceptance_satisfied: true,
            required_reviews_satisfied: true,
        })
    }
}

struct Fixture {
    _temp: TempDir,
    project: PathBuf,
    layout: ProjectLayout,
}

fn command(cwd: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run fixture command");
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 fixture output")
}

fn project(source: &str, with_rule: bool, initialize_store: bool) -> Fixture {
    let temp = TempDir::new().expect("temp project");
    let project = temp.path().join("project");
    fs::create_dir(&project).expect("project root");
    command(&project, "git", &["init", "--quiet"]);
    fs::create_dir(project.join("src")).expect("source directory");
    fs::write(project.join("src/app.py"), source).expect("source fixture");
    if with_rule {
        fs::create_dir_all(project.join("whetstone/rules/python")).expect("rules directory");
        fs::write(
            project.join("whetstone/rules/python/names.yaml"),
            r#"source:
  name: team
rules:
  - id: team.lowercase-functions
    severity: must
    confidence: high
    category: convention
    description: Function names must begin with a lowercase character.
    source_url: https://example.com/team/functions
    approved: true
    status: approved
    signals:
      - id: uppercase-function
        strategy: ast
        description: Finds uppercase function names.
        weight: required
        ast_query: '((function_definition name: (identifier) @match) (#match? @match "^[A-Z]"))'
    golden_examples:
      - code: "def read_config():\n    pass\n"
        verdict: pass
        reason: Lowercase names comply with the rule.
      - code: "def ReadConfig():\n    pass\n"
        verdict: fail
        reason: Uppercase names violate the rule.
"#,
        )
        .expect("rule fixture");
    }
    command(&project, "git", &["add", "."]);
    command(
        &project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    let layout = ProjectLayout::resolve(&project, None).expect("layout");
    if initialize_store {
        DoltRepository::initialize(&layout.store_path(StoreKind::Private), StoreKind::Private)
            .expect("private repair store");
    }
    Fixture {
        _temp: temp,
        project,
        layout,
    }
}

fn context() -> RepairTaskContext {
    RepairTaskContext {
        objective: "Rename the violating function without changing behavior.".into(),
        non_goals: vec!["Do not change policy, tests, configuration, or release state.".into()],
        applicable_guidance: Vec::new(),
        allowed_paths: vec!["src/app.py".into()],
        excluded_paths: vec!["tests".into(), "whetstone".into()],
        check_paths: vec!["src/app.py".into()],
        check_language: Some("python".into()),
        required_rules: Vec::new(),
        final_check_paths: vec!["src".into()],
        final_check_language: Some("python".into()),
        final_required_rules: Vec::new(),
        budget: RepairBudget {
            max_attempts: 4,
            max_repeated_finding: 3,
            max_elapsed_seconds: 1_200,
            max_resource_units: 100,
        },
    }
}

fn begin_request(
    session: &str,
    request_id: &str,
    context: RepairTaskContext,
) -> BeginRepairRequest {
    BeginRepairRequest {
        session_id: session.into(),
        request_id: request_id.into(),
        task: ExternalRef {
            system: ExternalSystem::Beads,
            stable_id: "task-42".into(),
            revision: Some("7".into()),
        },
        authority_revision: 7,
        authority_evidence: RepairAuthorityEvidence {
            locator: "host-grant:task-42".into(),
        },
        context,
        now: NOW.into(),
    }
}

fn checkpoint(
    session: &str,
    request_id: &str,
    revision: u64,
    kind: HostCheckpointKind,
) -> CheckpointRequest {
    CheckpointRequest {
        session_id: session.into(),
        request_id: request_id.into(),
        expected_revision: revision,
        authority_evidence: RepairAuthorityEvidence {
            locator: "host-grant:task-42".into(),
        },
        now: NOW.into(),
        kind,
    }
}

fn trusted_host() -> RepairHost<FixtureHostAdapter> {
    RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::Trusted,
    })
}

fn finalize(session: &str, request_id: &str, revision: u64) -> FinalizeRepairRequest {
    FinalizeRepairRequest {
        session_id: session.into(),
        request_id: request_id.into(),
        expected_revision: revision,
        authority_evidence: RepairAuthorityEvidence {
            locator: "host-grant:task-42".into(),
        },
        completion_evidence: RepairCompletionEvidence {
            locator: "host-acceptance:task-42".into(),
        },
        now: NOW.into(),
    }
}

fn current_handoff(fixture: &Fixture, session: &str) -> whetstone::domain::RecordRef {
    DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("open repair store")
    .all_records()
    .expect("records")
    .into_iter()
    .filter(|record| matches!(record.body, RecordBody::RepairHandoff(_)))
    .filter(|record| record.id.as_str().contains(session))
    .max_by_key(|record| record.revision)
    .expect("handoff exists")
    .reference()
    .expect("handoff ref")
}

fn session_record(fixture: &Fixture, session: &str) -> whetstone::domain::AgreementRecord {
    DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("open repair store")
    .latest(&RecordId::new(format!("repair.session.{session}")).expect("session id"))
    .expect("load session")
    .expect("session exists")
}

#[test]
fn post_edit_host_feedback_repairs_and_rechecks_the_exact_final_snapshot() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let initial = trusted_host()
        .begin(
            &fixture.project,
            begin_request("hook-flow", "hook-begin", context()),
        )
        .expect("begin repair session");
    assert_eq!(initial.state, RepairSessionStateRecord::Ready);
    assert_eq!(
        initial.lean_baseline_revision,
        "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48"
    );
    assert_eq!(initial.check.schema, RESPONSE_SCHEMA);
    assert_eq!(initial.check.state, ServiceState::Violated);
    assert!(initial.edit_authorized);
    assert!(!initial.findings.is_empty());
    assert_eq!(initial.outcome, "unknown");
    assert!(!initial.policy_change_authorized);
    assert!(!initial.publish_authorized);
    assert!(!initial.merge_authorized);
    assert!(!initial.release_authorized);

    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("authorized worker repair");

    // A fresh adapter instance models a host-process restart. Post-edit hook
    // feedback is returned synchronously to the same worker.
    let repaired = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "hook-flow",
                "hook-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("post-edit checkpoint");
    assert_eq!(repaired.checkpoint, HostCheckpointKind::PostEditHook);
    assert_eq!(
        repaired.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );
    assert_eq!(repaired.check.state, ServiceState::Success);
    assert!(!repaired.edit_authorized);
    assert_ne!(
        initial.check.required_snapshot, repaired.check.required_snapshot,
        "the successful receipt must bind the repaired code snapshot"
    );

    let completed = trusted_host()
        .finalize(
            &fixture.project,
            finalize("hook-flow", "hook-finalize", repaired.session.revision),
        )
        .expect("broader final verification");
    assert_eq!(completed.state, RepairSessionStateRecord::Verified);
    assert!(!completed.merge_authorized);
    assert!(!completed.release_authorized);
    assert_eq!(completed.outcome, "unknown");

    let persisted = session_record(&fixture, "hook-flow");
    let RecordBody::RepairSession(session) = persisted.body else {
        panic!("expected repair session");
    };
    assert_eq!(persisted.revision, 3);
    assert_eq!(session.attempts.len(), 1);
    assert!(session.total_elapsed_seconds >= 1);
    assert_eq!(session.total_resource_units, 2);
    assert!(session.final_verification_elapsed_seconds >= 1);
    assert_eq!(session.final_verification_resource_units, 1);
    assert_eq!(session.state, RepairSessionStateRecord::Verified);
    assert_eq!(
        session.lean_baseline_revision,
        "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48"
    );
    assert!(session.current_finding_ids.is_empty());
    assert_eq!(session.non_goals, context().non_goals);
    assert_eq!(
        command(&fixture.project, "git", &["rev-list", "--count", "HEAD"]),
        "1\n"
    );
    assert!(command(&fixture.project, "git", &["status", "--porcelain"]).contains("src/app.py"));
}

#[test]
fn humans_get_the_same_actionable_check_without_implicitly_authorizing_an_agent() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let human = CommandService.execute(ServiceRequest::Check(CheckRequest {
        project_dir: fixture.project.clone(),
        request_id: Some("human-check".into()),
        paths: vec![PathBuf::from("src/app.py")],
        language: Some("python".into()),
        rules: Vec::new(),
    }));
    assert_eq!(human.state, ServiceState::Violated);
    assert_eq!(
        human.data["report"]["results"][0]["findings"][0]["rule_id"],
        "team.lowercase-functions"
    );
    assert!(human
        .permitted_actions
        .iter()
        .any(|action| action.contains("repair within the current task scope")));
    let before_authority = DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("store")
    .all_records()
    .expect("records");
    assert!(!before_authority
        .iter()
        .any(|record| matches!(record.body, RecordBody::RepairSession(_))));

    let authorized = trusted_host()
        .begin(
            &fixture.project,
            begin_request("separate-edit-grant", "agent-begin", context()),
        )
        .expect("separately authorized session");
    assert!(authorized.edit_authorized);
}

#[test]
fn explicit_checkpoint_is_the_visible_fallback_for_hosts_without_hooks() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let initial = trusted_host()
        .begin(
            &fixture.project,
            begin_request("explicit-flow", "explicit-begin", context()),
        )
        .expect("begin");
    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair");
    let repaired = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "explicit-flow",
                "explicit-checkpoint",
                initial.session.revision,
                HostCheckpointKind::ExplicitCheckpoint,
            ),
        )
        .expect("explicit checkpoint");
    assert_eq!(repaired.checkpoint, HostCheckpointKind::ExplicitCheckpoint);
    assert_eq!(
        repaired.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );
}

#[test]
fn missing_or_expanded_authority_stops_before_work_or_storage_creation() {
    let missing = project("def ReadConfig():\n    pass\n", true, false);
    let error = RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::Missing,
    })
    .begin(
        &missing.project,
        begin_request("missing", "missing-begin", context()),
    )
    .expect_err("missing authority must fail");
    assert!(matches!(error, RepairHostError::MissingAuthority));
    assert!(!missing.layout.store_path(StoreKind::Private).exists());

    let expanded = project("def ReadConfig():\n    pass\n", true, true);
    let error = RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::ExpandedCapability,
    })
    .begin(
        &expanded.project,
        begin_request("expanded", "expanded-begin", context()),
    )
    .expect_err("ordinary repair cannot carry policy capability");
    assert!(matches!(error, RepairHostError::ForbiddenCapability));
    assert!(DoltRepository::open_existing(
        &expanded.layout.store_path(StoreKind::Private),
        StoreKind::Private
    )
    .expect("store")
    .all_records()
    .expect("records")
    .is_empty());
}

#[test]
fn unavailable_checker_hands_back_one_exact_resumable_decision() {
    let fixture = project("def ReadConfig():\n    pass\n", false, true);
    let feedback = trusted_host()
        .begin(
            &fixture.project,
            begin_request("unavailable", "unavailable-begin", context()),
        )
        .expect("unavailable is a recorded outcome");
    assert_eq!(feedback.state, RepairSessionStateRecord::NeedsDecision);
    assert!(matches!(
        feedback.check.state,
        ServiceState::Unknown | ServiceState::Unavailable
    ));
    assert!(!feedback.edit_authorized);
    assert_eq!(feedback.findings.len(), 1);
    let handoff_ref = current_handoff(&fixture, "unavailable");
    trusted_host()
        .validate_handoff_reply(
            &fixture.project,
            &handoff_ref,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "unavailable-handoff-check",
        )
        .expect("exact handoff target");
    let records = DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("store")
    .all_records()
    .expect("records");
    let handoffs = records
        .iter()
        .filter(|record| matches!(record.body, RecordBody::RepairHandoff(_)))
        .collect::<Vec<_>>();
    assert_eq!(handoffs.len(), 1);
    let RecordBody::RepairHandoff(handoff) = &handoffs[0].body else {
        unreachable!()
    };
    assert_eq!(handoff.expected_session_revision, 1);
    assert_eq!(handoff.stable_finding_ids, feedback.findings);
    assert!(!handoff.question.is_empty());
    assert!(!handoff.recommendation.is_empty());
    assert!(!handoff.alternatives.is_empty());
    assert!(!handoff.impact.is_empty());
    assert!(!handoff.evidence.is_empty());
    assert!(handoff.permitted_next_step.contains("wh change"));
}

#[test]
fn restart_keeps_attempt_time_resource_budgets_and_no_progress_is_terminal() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("restart", "restart-begin", context()),
        )
        .expect("begin");
    let first = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "restart",
                "restart-first",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("first failed attempt");
    assert_eq!(first.state, RepairSessionStateRecord::Ready);

    let stopped = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "restart",
                "restart-second",
                2,
                HostCheckpointKind::ExplicitCheckpoint,
            ),
        )
        .expect("no progress becomes a handoff, not an unrecorded error");
    assert_eq!(stopped.state, RepairSessionStateRecord::NeedsDecision);
    assert!(!stopped.edit_authorized);
    let persisted = session_record(&fixture, "restart");
    let RecordBody::RepairSession(session) = persisted.body else {
        panic!("repair session");
    };
    assert_eq!(persisted.revision, 3);
    assert_eq!(session.attempts.len(), 2);
    assert!(session.total_elapsed_seconds >= 2);
    assert_eq!(session.total_resource_units, 2);
    let handoff_ref = current_handoff(&fixture, "restart");
    trusted_host()
        .validate_handoff_reply(
            &fixture.project,
            &handoff_ref,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "restart-handoff-check",
        )
        .expect("current handoff response");
    let error = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "restart",
                "restart-third",
                3,
                HostCheckpointKind::ExplicitCheckpoint,
            ),
        )
        .expect_err("terminal handoff cannot silently restart");
    assert!(matches!(error, RepairHostError::Terminal));
}

#[test]
fn elapsed_or_resource_budget_exhaustion_survives_restart() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let mut begin = begin_request("budget", "budget-begin", context());
    begin.context.budget.max_attempts = 5;
    begin.context.budget.max_repeated_finding = 5;
    begin.context.budget.max_elapsed_seconds = 1_200;
    begin.context.budget.max_resource_units = 1;
    trusted_host()
        .begin(&fixture.project, begin)
        .expect("begin");
    let stopped = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "budget",
                "budget-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("budget stop");
    assert_eq!(stopped.state, RepairSessionStateRecord::NeedsDecision);
    let persisted = session_record(&fixture, "budget");
    let RecordBody::RepairSession(session) = persisted.body else {
        panic!("repair session");
    };
    assert!(session.total_elapsed_seconds >= 1);
    assert_eq!(session.total_resource_units, 1);
    assert_eq!(session.attempts.len(), 1);
}

#[test]
fn an_expired_wall_clock_budget_stops_before_running_an_attempt() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::AdvanceTo(100),
        },
        clock.clone(),
    );
    let mut request = begin_request("wall-clock", "wall-clock-begin", context());
    request.context.budget.max_elapsed_seconds = 1;
    host.begin(&fixture.project, request).expect("begin");
    clock.set(101);
    let request = checkpoint(
        "wall-clock",
        "wall-clock-checkpoint",
        1,
        HostCheckpointKind::PostEditHook,
    );
    let stopped = host
        .checkpoint(&fixture.project, request.clone())
        .expect("expired budget becomes a handoff");
    assert_eq!(stopped.state, RepairSessionStateRecord::NeedsDecision);
    assert_eq!(stopped.check.data["reason_code"], "repair_budget_exhausted");
    let RecordBody::RepairSession(session) = session_record(&fixture, "wall-clock").body else {
        panic!("session");
    };
    assert!(session.attempts.is_empty());
    assert_eq!(session.total_resource_units, 0);
    let replayed = host
        .checkpoint(&fixture.project, request)
        .expect("budget-stop response replay");
    assert_eq!(replayed, stopped);
    let RecordBody::RepairSession(replayed_session) = session_record(&fixture, "wall-clock").body
    else {
        panic!("session");
    };
    assert_eq!(replayed_session.total_resource_units, 0);
    assert!(replayed_session.attempts.is_empty());
}

#[test]
fn a_green_final_permitted_repair_attempt_advances_to_final_verification() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let mut repair_context = context();
    repair_context.budget.max_attempts = 1;
    repair_context.budget.max_repeated_finding = 1;
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("last-attempt", "last-attempt-begin", repair_context),
        )
        .expect("begin");
    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair on final permitted attempt");
    let feedback = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "last-attempt",
                "last-attempt-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("green final permitted attempt");
    assert_eq!(
        feedback.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );
}

#[test]
fn alternating_failed_candidates_are_detected_as_oscillation() {
    let fixture = project("# a\ndef ReadConfig():\n    pass\n", true, true);
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("oscillation", "oscillation-begin", context()),
        )
        .expect("begin");
    for (revision, marker) in [(1, "b"), (2, "a"), (3, "b")] {
        fs::write(
            fixture.project.join("src/app.py"),
            format!("# {marker}\ndef ReadConfig():\n    pass\n"),
        )
        .expect("candidate");
        let feedback = trusted_host()
            .checkpoint(
                &fixture.project,
                checkpoint(
                    "oscillation",
                    &format!("oscillation-{revision}"),
                    revision,
                    HostCheckpointKind::PostEditHook,
                ),
            )
            .expect("checkpoint");
        if revision < 3 {
            assert_eq!(feedback.state, RepairSessionStateRecord::Ready);
        } else {
            assert_eq!(feedback.state, RepairSessionStateRecord::NeedsDecision);
        }
    }
    let persisted = session_record(&fixture, "oscillation");
    let RecordBody::RepairSession(session) = persisted.body else {
        panic!("repair session");
    };
    assert_eq!(session.attempts.len(), 3);
    assert!(session.total_elapsed_seconds >= 3);
    assert_eq!(session.state, RepairSessionStateRecord::NeedsDecision);
    assert!(session
        .attempts
        .windows(2)
        .all(|pair| pair[0].finding_ids == pair[1].finding_ids));
}

#[test]
fn policy_tests_config_baselines_scope_and_symlink_escapes_cannot_be_greenwashed() {
    for (index, path) in [
        "whetstone/rules/python/names.yaml",
        "tests/failing.py",
        "ruff.toml",
        "src/.ruff.toml",
        "crates/widget/.cargo/config.toml",
        "crates/widget/.github/workflows/check.yml",
        "crates/widget/.githooks/pre-commit",
        "crates/widget/AGENTS.md",
        "crates/widget/SKILL.md",
        "pytest.ini",
        "conftest.py",
        "tsconfig.json",
        "vitest.config.ts",
        "score.baseline",
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = project("def ReadConfig():\n    pass\n", true, true);
        let mut broad = context();
        broad.allowed_paths = vec![".".into()];
        broad.excluded_paths = Vec::new();
        let session = format!("protected-{index}");
        trusted_host()
            .begin(
                &fixture.project,
                begin_request(&session, &format!("{session}-begin"), broad),
            )
            .expect("begin broad source task");
        if let Some(parent) = fixture.project.join(path).parent() {
            fs::create_dir_all(parent).expect("protected parent");
        }
        fs::write(fixture.project.join(path), "weakened\n").expect("protected mutation");
        let request = checkpoint(
            &session,
            &format!("protected-{path}"),
            1,
            HostCheckpointKind::PostEditHook,
        );
        let error = trusted_host()
            .checkpoint(&fixture.project, request)
            .expect_err("protected path");
        assert!(matches!(error, RepairHostError::ProtectedPath(_)), "{path}");
        assert_eq!(session_record(&fixture, &session).revision, 1);
    }

    // Broad authority above permits README, so explicitly demonstrate a
    // narrower authenticated scope in a separate session.
    let second = project("def ReadConfig():\n    pass\n", true, true);
    trusted_host()
        .begin(
            &second.project,
            begin_request("narrow", "narrow-begin", context()),
        )
        .expect("narrow begin");
    fs::write(second.project.join("README.md"), "outside task\n").expect("readme");
    let request = checkpoint(
        "narrow",
        "narrow-checkpoint",
        1,
        HostCheckpointKind::ExplicitCheckpoint,
    );
    assert!(matches!(
        trusted_host().checkpoint(&second.project, request),
        Err(RepairHostError::InvalidPath(_))
    ));

    let embedded = project(
        "def test_embedded():\n    pass\n\ndef ReadConfig():\n    pass\n",
        true,
        true,
    );
    trusted_host()
        .begin(
            &embedded.project,
            begin_request("embedded", "embedded-begin", context()),
        )
        .expect("embedded-test baseline");
    fs::write(
        embedded.project.join("src/app.py"),
        "def ReadConfig():\n    pass\n",
    )
    .expect("remove embedded test");
    assert!(matches!(
        trusted_host().checkpoint(
            &embedded.project,
            checkpoint(
                "embedded",
                "embedded-test-removal",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        ),
        Err(RepairHostError::ProtectedPath(_))
    ));

    let mixed_tests = project("def ReadConfig():\n    pass\n", true, true);
    fs::write(
        mixed_tests.project.join("src/runtime.rs"),
        "#[\n    tokio::test\n]\nasync fn exercises_runtime() {}\n",
    )
    .expect("async Rust test");
    fs::write(
        mixed_tests.project.join("src/widget.ts"),
        "    describe ('widget', () => { test ('works', () => {}); });\n",
    )
    .expect("indented JavaScript test");
    command(&mixed_tests.project, "git", &["add", "."]);
    command(
        &mixed_tests.project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "mixed tests",
        ],
    );
    let mut broad = context();
    broad.allowed_paths = vec![".".into()];
    broad.excluded_paths = Vec::new();
    trusted_host()
        .begin(
            &mixed_tests.project,
            begin_request("mixed-tests", "mixed-tests-begin", broad),
        )
        .expect("mixed test baseline");
    for (request_id, path) in [
        ("remove-rust-test", "src/runtime.rs"),
        ("remove-js-test", "src/widget.ts"),
    ] {
        let original = fs::read_to_string(mixed_tests.project.join(path)).expect("test source");
        fs::write(mixed_tests.project.join(path), "production_only();\n")
            .expect("remove embedded test");
        assert!(matches!(
            trusted_host().checkpoint(
                &mixed_tests.project,
                checkpoint(
                    "mixed-tests",
                    request_id,
                    1,
                    HostCheckpointKind::PostEditHook,
                ),
            ),
            Err(RepairHostError::ProtectedPath(_))
        ));
        fs::write(mixed_tests.project.join(path), original).expect("restore protected test");
    }

    let ordinary_calls = project("def ReadConfig():\n    pass\n", true, true);
    fs::write(
        ordinary_calls.project.join("src/runtime.ts"),
        "value.split(','); process.exit(0); repo.commit(); /ready/.test (value);\n",
    )
    .expect("ordinary source calls");
    command(&ordinary_calls.project, "git", &["add", "."]);
    command(
        &ordinary_calls.project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "ordinary calls",
        ],
    );
    let mut ordinary_context = context();
    ordinary_context.allowed_paths = vec!["src".into()];
    trusted_host()
        .begin(
            &ordinary_calls.project,
            begin_request("ordinary-calls", "ordinary-calls-begin", ordinary_context),
        )
        .expect("ordinary call baseline");
    fs::write(
        ordinary_calls.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair ordinary-call fixture");
    fs::write(
        ordinary_calls.project.join("src/runtime.ts"),
        "input.split(':'); process.exit(1); transaction.commit(); /done/.test (input);\n",
    )
    .expect("edit ordinary source calls");
    let ordinary_feedback = trusted_host()
        .checkpoint(
            &ordinary_calls.project,
            checkpoint(
                "ordinary-calls",
                "ordinary-calls-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("ordinary calls must not be mistaken for embedded tests");
    assert_eq!(
        ordinary_feedback.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );

    let nested = project("def ReadConfig():\n    pass\n", true, true);
    let nested_repo = nested.project.join("vendor/component");
    fs::create_dir_all(&nested_repo).expect("nested repository");
    command(&nested_repo, "git", &["init", "--quiet"]);
    fs::write(
        nested_repo.join("test_component.py"),
        "def test_component(): pass\n",
    )
    .expect("nested test");
    command(&nested_repo, "git", &["add", "."]);
    command(
        &nested_repo,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "nested fixture",
        ],
    );
    command(&nested.project, "git", &["add", "vendor/component"]);
    command(
        &nested.project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "nested repository",
        ],
    );
    fs::write(
        nested_repo.join("test_component.py"),
        "def test_component(): assert False\n",
    )
    .expect("dirty nested test before session");
    let mut broad = context();
    broad.allowed_paths = vec![".".into()];
    broad.excluded_paths = Vec::new();
    trusted_host()
        .begin(
            &nested.project,
            begin_request("nested-dirty", "nested-dirty-begin", broad),
        )
        .expect("dirty nested baseline");
    fs::write(
        nested_repo.join("test_component.py"),
        "def test_component(): assert True\n",
    )
    .expect("change already-dirty nested test");
    assert!(matches!(
        trusted_host().checkpoint(
            &nested.project,
            checkpoint(
                "nested-dirty",
                "nested-dirty-check",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        ),
        Err(RepairHostError::ProtectedPath(_))
    ));

    let ignored_source = project("def ReadConfig():\n    pass\n", true, true);
    fs::write(ignored_source.project.join(".gitignore"), "scratch.py\n").expect("ignore file");
    command(&ignored_source.project, "git", &["add", ".gitignore"]);
    command(
        &ignored_source.project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "ignore fixture",
        ],
    );
    trusted_host()
        .begin(
            &ignored_source.project,
            begin_request("ignored-source", "ignored-source-begin", context()),
        )
        .expect("ignored source baseline");
    fs::write(ignored_source.project.join("scratch.py"), "outside scope\n")
        .expect("ignored out-of-scope source");
    assert!(matches!(
        trusted_host().checkpoint(
            &ignored_source.project,
            checkpoint(
                "ignored-source",
                "ignored-source-check",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        ),
        Err(RepairHostError::InvalidPath(_))
    ));

    let ignored_test = project("def ReadConfig():\n    pass\n", true, true);
    fs::write(ignored_test.project.join(".gitignore"), "private-tests/\n")
        .expect("ignore private tests");
    fs::create_dir(ignored_test.project.join("private-tests")).expect("private tests dir");
    fs::write(
        ignored_test.project.join("private-tests/test_hidden.py"),
        "def test_hidden():\n    assert False\n",
    )
    .expect("ignored failing test");
    command(&ignored_test.project, "git", &["add", ".gitignore"]);
    command(
        &ignored_test.project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "ignore private tests",
        ],
    );
    let mut broad = context();
    broad.allowed_paths = vec![".".into()];
    broad.excluded_paths = Vec::new();
    trusted_host()
        .begin(
            &ignored_test.project,
            begin_request("ignored-test", "ignored-test-begin", broad),
        )
        .expect("ignored test baseline");
    fs::write(
        ignored_test.project.join("private-tests/test_hidden.py"),
        "def test_hidden():\n    assert True\n",
    )
    .expect("weaken ignored test");
    assert!(matches!(
        trusted_host().checkpoint(
            &ignored_test.project,
            checkpoint(
                "ignored-test",
                "ignored-test-check",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        ),
        Err(RepairHostError::ProtectedPath(_))
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let symlink_fixture = project("def ReadConfig():\n    pass\n", true, true);
        trusted_host()
            .begin(
                &symlink_fixture.project,
                begin_request("symlink", "symlink-begin", context()),
            )
            .expect("symlink begin");
        let outside = symlink_fixture._temp.path().join("outside.py");
        fs::write(&outside, "def ReadConfig(): pass\n").expect("outside");
        symlink(&outside, symlink_fixture.project.join("src/escape.py")).expect("symlink");
        let request = checkpoint(
            "symlink",
            "protected-symlink",
            1,
            HostCheckpointKind::PostEditHook,
        );
        assert!(matches!(
            trusted_host().checkpoint(&symlink_fixture.project, request),
            Err(RepairHostError::InvalidPath(_))
        ));
    }
}

#[cfg(unix)]
#[test]
fn chmod_only_change_to_protected_hook_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let hook = fixture.project.join(".githooks/pre-push");
    fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks directory");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").expect("write hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("make hook executable");

    let mut broad = context();
    broad.allowed_paths = vec![".".into()];
    broad.excluded_paths = Vec::new();
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("protected-chmod", "protected-chmod-begin", broad),
        )
        .expect("begin with executable hook");

    fs::set_permissions(&hook, fs::Permissions::from_mode(0o644)).expect("disable hook");
    let error = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "protected-chmod",
                "protected-chmod-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect_err("chmod-only protected mutation");
    assert!(matches!(error, RepairHostError::ProtectedPath(path) if path == ".githooks/pre-push"));
    assert_eq!(session_record(&fixture, "protected-chmod").revision, 1);
}

#[cfg(unix)]
#[test]
fn chmod_only_change_to_protected_control_directory_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let hooks = fixture.project.join(".githooks");
    fs::create_dir_all(&hooks).expect("create hooks directory");
    fs::set_permissions(&hooks, fs::Permissions::from_mode(0o755))
        .expect("set protected directory mode");

    let mut broad = context();
    broad.allowed_paths = vec![".".into()];
    broad.excluded_paths = Vec::new();
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("protected-dir-chmod", "protected-dir-chmod-begin", broad),
        )
        .expect("begin with protected directory");

    fs::set_permissions(&hooks, fs::Permissions::from_mode(0o777))
        .expect("make protected directory world-writable");
    let error = trusted_host()
        .checkpoint(
            &fixture.project,
            checkpoint(
                "protected-dir-chmod",
                "protected-dir-chmod-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect_err("chmod-only protected directory mutation");
    assert!(matches!(error, RepairHostError::ProtectedPath(path) if path == ".githooks"));
    assert_eq!(session_record(&fixture, "protected-dir-chmod").revision, 1);
}

#[test]
fn cancellation_is_persisted_and_a_stored_session_never_becomes_authority() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("cancel", "cancel-begin", context()),
        )
        .expect("begin");

    let missing = RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::Missing,
    })
    .checkpoint(
        &fixture.project,
        checkpoint(
            "cancel",
            "missing-authority-resume",
            1,
            HostCheckpointKind::ExplicitCheckpoint,
        ),
    )
    .expect_err("persisted identity fields cannot substitute for host authority");
    assert!(matches!(missing, RepairHostError::MissingAuthority));

    let cancelled = trusted_host()
        .cancel(
            &fixture.project,
            "cancel",
            1,
            "cancel-request".into(),
            NOW.into(),
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
        )
        .expect("cancel");
    let replayed_cancel = trusted_host()
        .cancel(
            &fixture.project,
            "cancel",
            1,
            "cancel-request".into(),
            NOW.into(),
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
        )
        .expect("lost cancel response is replayable");
    assert_eq!(replayed_cancel, cancelled);
    let persisted = session_record(&fixture, "cancel");
    let mut forged = persisted.clone();
    forged.provenance.authority = ProvenanceAuthority::OwnerAuthored;
    assert!(matches!(
        forged.validate(),
        Err(DomainError::OperationalRecordCannotGrantAuthority)
    ));
    let RecordBody::RepairSession(session) = persisted.body else {
        panic!("repair session");
    };
    assert_eq!(session.state, RepairSessionStateRecord::Cancelled);

    let error = RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::Missing,
    })
    .checkpoint(
        &fixture.project,
        checkpoint(
            "cancel",
            "cancel-resume",
            2,
            HostCheckpointKind::ExplicitCheckpoint,
        ),
    )
    .expect_err("persisted JSON cannot grant authority");
    assert!(matches!(error, RepairHostError::Terminal));
}

#[test]
fn a_green_fast_check_does_not_complete_without_task_acceptance_and_review() {
    let fixture = project("def read_config():\n    pass\n", true, true);
    let initial = trusted_host()
        .begin(
            &fixture.project,
            begin_request("final-gate", "final-begin", context()),
        )
        .expect("green repair candidate");
    assert_eq!(
        initial.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );

    let rejected = RepairHost::new(FixtureHostAdapter {
        mode: VerifierMode::CompletionRejected,
    })
    .finalize(
        &fixture.project,
        finalize("final-gate", "final-rejected", initial.session.revision),
    )
    .expect("rejected completion becomes an accountable handoff");
    assert_eq!(rejected.state, RepairSessionStateRecord::NeedsDecision);
    assert!(!rejected.edit_authorized);
    assert!(!rejected.merge_authorized);
    assert!(!rejected.release_authorized);
    assert_eq!(rejected.outcome, "unknown");
    assert!(rejected
        .findings
        .iter()
        .any(|finding| finding.starts_with("completion:")));
    let returned_handoff = rejected
        .handoff
        .as_ref()
        .expect("needs-decision feedback includes the persisted handoff");
    assert!(!returned_handoff.question.is_empty());
    assert!(!returned_handoff.recommendation.is_empty());
    assert!(!returned_handoff.alternatives.is_empty());
    assert!(!returned_handoff.impact.is_empty());
    assert!(!returned_handoff.evidence.is_empty());
    assert!(!returned_handoff.permitted_next_step.is_empty());
    assert!(matches!(
        session_record(&fixture, "final-gate").body,
        RecordBody::RepairSession(ref session)
            if session.state == RepairSessionStateRecord::NeedsDecision
    ));
    let handoff = current_handoff(&fixture, "final-gate");
    trusted_host()
        .validate_handoff_reply(
            &fixture.project,
            &handoff,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "final-handoff-check",
        )
        .expect("unchanged final-phase handoff remains valid");
}

#[test]
fn final_verification_rejects_policy_changes_after_the_fast_gate() {
    let fixture = project("def read_config():\n    pass\n", true, true);
    let initial = trusted_host()
        .begin(
            &fixture.project,
            begin_request("final-policy", "final-policy-begin", context()),
        )
        .expect("green fast gate");
    assert_eq!(
        initial.state,
        RepairSessionStateRecord::ReadyForFinalVerification
    );
    fs::write(
        fixture.project.join("whetstone/rules/python/names.yaml"),
        "weakened: true\n",
    )
    .expect("policy mutation");
    let error = trusted_host()
        .finalize(
            &fixture.project,
            finalize(
                "final-policy",
                "final-policy-finalize",
                initial.session.revision,
            ),
        )
        .expect_err("final gate must bind immutable policy inputs");
    assert!(matches!(
        error,
        RepairHostError::WorkspaceChanged | RepairHostError::ProtectedPath(_)
    ));
    assert_eq!(session_record(&fixture, "final-policy").revision, 1);
}

#[test]
fn final_completion_counts_delay_and_rejects_expired_authority_or_budget() {
    for (session, completed_at, expires_at, max_elapsed, expected_finding) in [
        (
            "final-expiry",
            201,
            200,
            500,
            "authority:expired-or-revoked-during-completion",
        ),
        (
            "final-budget",
            151,
            1_000,
            50,
            "budget:exhausted-during-final-verification",
        ),
    ] {
        let fixture = project("def read_config():\n    pass\n", true, true);
        let clock = FakeClock::new(100);
        let host = RepairHost::with_clock(
            BoundaryAdapter {
                clock: clock.clone(),
                expires_at_unix: expires_at,
                completion: CompletionBoundary::AdvanceTo(completed_at),
            },
            clock,
        );
        let mut repair_context = context();
        repair_context.budget.max_elapsed_seconds = max_elapsed;
        let initial = host
            .begin(
                &fixture.project,
                begin_request(session, &format!("{session}-begin"), repair_context),
            )
            .expect("green fast gate");
        let result = host
            .finalize(
                &fixture.project,
                finalize(
                    session,
                    &format!("{session}-finalize"),
                    initial.session.revision,
                ),
            )
            .expect("boundary failure persists a handoff");
        assert_eq!(result.state, RepairSessionStateRecord::NeedsDecision);
        assert_eq!(result.findings, vec![expected_finding.to_string()]);
        assert!(!result.merge_authorized);
        assert!(!result.release_authorized);
        let RecordBody::RepairSession(record) = session_record(&fixture, session).body else {
            panic!("session");
        };
        assert_eq!(record.total_elapsed_seconds, completed_at - 100);
        assert_eq!(
            record.final_verification_elapsed_seconds,
            completed_at - 100
        );
        assert_eq!(record.state, RepairSessionStateRecord::NeedsDecision);
    }
}

#[test]
fn final_completion_rechecks_the_candidate_after_host_attestation() {
    let fixture = project("def read_config():\n    pass\n", true, true);
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::Mutate(fixture.project.join("src/app.py")),
        },
        clock,
    );
    let initial = host
        .begin(
            &fixture.project,
            begin_request("final-mutation", "final-mutation-begin", context()),
        )
        .expect("green fast gate");
    let result = host
        .finalize(
            &fixture.project,
            finalize(
                "final-mutation",
                "final-mutation-finalize",
                initial.session.revision,
            ),
        )
        .expect("candidate mutation persists a handoff");
    assert_eq!(result.state, RepairSessionStateRecord::NeedsDecision);
    assert_eq!(
        result.findings,
        vec!["workspace:changed-during-completion".to_string()]
    );
    let handoff = current_handoff(&fixture, "final-mutation");
    assert!(matches!(
        host.validate_handoff_reply(
            &fixture.project,
            &handoff,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "final-mutation-handoff",
        ),
        Err(RepairHostError::WorkspaceChanged)
    ));
}

#[cfg(unix)]
#[test]
fn final_completion_rejects_a_mode_change_during_host_attestation() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = project("def read_config():\n    pass\n", true, true);
    let hook = fixture.project.join(".githooks/pre-push");
    fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks directory");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").expect("write hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("make hook executable");
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::ChangeMode {
                path: hook,
                mode: 0o644,
            },
        },
        clock,
    );
    let initial = host
        .begin(
            &fixture.project,
            begin_request("final-mode", "final-mode-begin", context()),
        )
        .expect("green fast gate");
    let result = host
        .finalize(
            &fixture.project,
            finalize(
                "final-mode",
                "final-mode-finalize",
                initial.session.revision,
            ),
        )
        .expect("mode mutation persists a handoff");
    assert_eq!(result.state, RepairSessionStateRecord::NeedsDecision);
    assert_eq!(
        result.findings,
        vec!["workspace:changed-during-completion".to_string()]
    );
}

#[cfg(unix)]
#[test]
fn final_completion_rejects_a_directory_mode_change_during_host_attestation() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = project("def read_config():\n    pass\n", true, true);
    let source_directory = fixture.project.join("src");
    fs::set_permissions(&source_directory, fs::Permissions::from_mode(0o755))
        .expect("set source directory mode");
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::ChangeMode {
                path: source_directory,
                mode: 0o777,
            },
        },
        clock,
    );
    let initial = host
        .begin(
            &fixture.project,
            begin_request("final-dir-mode", "final-dir-mode-begin", context()),
        )
        .expect("green fast gate");
    let result = host
        .finalize(
            &fixture.project,
            finalize(
                "final-dir-mode",
                "final-dir-mode-finalize",
                initial.session.revision,
            ),
        )
        .expect("directory mode mutation persists a handoff");
    assert_eq!(result.state, RepairSessionStateRecord::NeedsDecision);
    assert_eq!(
        result.findings,
        vec!["workspace:changed-during-completion".to_string()]
    );
}

#[test]
fn a_new_session_id_cannot_reset_the_same_authority_budget() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("first-budget", "first-budget-begin", context()),
        )
        .expect("first session");
    let error = trusted_host()
        .begin(
            &fixture.project,
            begin_request("reset-budget", "reset-budget-begin", context()),
        )
        .expect_err("same task authority revision cannot mint another budget");
    assert!(matches!(error, RepairHostError::DuplicateAuthoritySession));
}

#[test]
fn concurrent_begins_atomically_reserve_one_task_authority_budget() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let barrier = Arc::new(Barrier::new(2));
    let initial_calls = Arc::new(AtomicU64::new(0));
    let project = fixture.project.clone();
    let handles = ["race-a", "race-b"].map(|session| {
        let project = project.clone();
        let barrier = barrier.clone();
        let initial_calls = initial_calls.clone();
        std::thread::spawn(move || {
            RepairHost::new(BarrierAdapter {
                barrier,
                initial_calls,
            })
            .begin(
                &project,
                begin_request(session, &format!("{session}-begin"), context()),
            )
        })
    });
    let results = handles.map(|handle| handle.join().expect("begin thread"));
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepairHostError::DuplicateAuthoritySession)))
            .count(),
        1
    );
    let records = DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("store")
    .all_records()
    .expect("records");
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.body, RecordBody::RepairSession(_)))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.body, RecordBody::RepairAuthorityReservation(_)))
            .count(),
        2,
        "one claimed and one session-bound reservation revision"
    );
}

#[test]
fn identical_concurrent_begins_execute_one_bootstrap_check_pair() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let barrier = Arc::new(Barrier::new(2));
    let initial_calls = Arc::new(AtomicU64::new(0));
    let check_calls = Arc::new(AtomicU64::new(0));
    let project = fixture.project.clone();
    let handles = [(), ()].map(|()| {
        let project = project.clone();
        let barrier = barrier.clone();
        let initial_calls = initial_calls.clone();
        let check_calls = check_calls.clone();
        std::thread::spawn(move || {
            RepairHost::with_clock_and_executor(
                BarrierAdapter {
                    barrier,
                    initial_calls,
                },
                FakeClock::new(100),
                CountingCheckExecutor(check_calls),
            )
            .begin(
                &project,
                begin_request("same-race", "same-race-begin", context()),
            )
        })
    });
    let results = handles.map(|handle| handle.join().expect("begin thread"));
    assert!(results.iter().all(|result| {
        result.is_ok() || matches!(result, Err(RepairHostError::OperationInProgress))
    }));
    assert!(results.iter().any(Result::is_ok));
    assert_eq!(
        check_calls.load(Ordering::SeqCst),
        2,
        "only the exclusive reservation owner may run the fast and final-baseline checks"
    );
}

#[test]
fn concurrent_checkpoints_execute_under_one_durable_operation_claim() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let check_calls = Arc::new(AtomicU64::new(0));
    let clock = FakeClock::new(100);
    let executor = CountingCheckExecutor(check_calls.clone());
    RepairHost::with_clock_and_executor(
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        },
        clock.clone(),
        executor.clone(),
    )
    .begin(
        &fixture.project,
        begin_request("checkpoint-race", "checkpoint-race-begin", context()),
    )
    .expect("begin");
    check_calls.store(0, Ordering::SeqCst);
    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair");
    let barrier = Arc::new(Barrier::new(2));
    let project = fixture.project.clone();
    let handles = [(), ()].map(|()| {
        let project = project.clone();
        let barrier = barrier.clone();
        let clock = clock.clone();
        let executor = executor.clone();
        std::thread::spawn(move || {
            barrier.wait();
            RepairHost::with_clock_and_executor(
                FixtureHostAdapter {
                    mode: VerifierMode::Trusted,
                },
                clock,
                executor,
            )
            .checkpoint(
                &project,
                checkpoint(
                    "checkpoint-race",
                    "checkpoint-race-same-request",
                    1,
                    HostCheckpointKind::PostEditHook,
                ),
            )
        })
    });
    let results = handles.map(|handle| handle.join().expect("checkpoint thread"));
    assert!(
        results.iter().all(|result| {
            result.is_ok()
                || matches!(
                    result,
                    Err(RepairHostError::OperationInProgress | RepairHostError::Stale { .. })
                )
        }),
        "duplicate request must replay or stop without work: {results:?}"
    );
    assert!(results.iter().any(Result::is_ok));
    assert_eq!(
        check_calls.load(Ordering::SeqCst),
        1,
        "an exact replay may return stored feedback but must not rerun the checker"
    );
    let records = DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("store")
    .all_records()
    .expect("records");
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.body, RecordBody::RepairOperationClaim(_)))
            .count(),
        1
    );
}

#[test]
fn cancellation_committed_before_operation_claim_prevents_checker_execution() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let check_calls = Arc::new(AtomicU64::new(0));
    let clock = FakeClock::new(100);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let host = RepairHost::with_clock_and_executor(
        PausingAuthorityAdapter {
            calls: Arc::new(AtomicU64::new(0)),
            pause_on_call: 3,
            entered: entered.clone(),
            release: release.clone(),
        },
        clock,
        CountingCheckExecutor(check_calls.clone()),
    );
    host.begin(
        &fixture.project,
        begin_request("cancel-race", "cancel-race-begin", context()),
    )
    .expect("begin");
    check_calls.store(0, Ordering::SeqCst);
    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair");

    let project = fixture.project.clone();
    let checkpoint_thread = std::thread::spawn(move || {
        host.checkpoint(
            &project,
            checkpoint(
                "cancel-race",
                "cancel-race-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
    });
    entered.wait();
    let cancelled = trusted_host()
        .cancel(
            &fixture.project,
            "cancel-race",
            1,
            "cancel-race-cancel".into(),
            NOW.into(),
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
        )
        .expect("cancellation wins before operation claim");
    assert_eq!(cancelled.revision, 2);
    release.wait();

    assert!(matches!(
        checkpoint_thread.join().expect("checkpoint thread"),
        Err(RepairHostError::Stale {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(
        check_calls.load(Ordering::SeqCst),
        0,
        "a cancellation ordered before the claim must prevent checker work"
    );
}

#[test]
fn identical_concurrent_finalizations_execute_one_final_check() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let check_calls = Arc::new(AtomicU64::new(0));
    let clock = FakeClock::new(100);
    let executor = CountingCheckExecutor(check_calls.clone());
    let host = RepairHost::with_clock_and_executor(
        FixtureHostAdapter {
            mode: VerifierMode::Trusted,
        },
        clock.clone(),
        executor.clone(),
    );
    host.begin(
        &fixture.project,
        begin_request("final-race", "final-race-begin", context()),
    )
    .expect("begin");
    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair");
    let checkpointed = host
        .checkpoint(
            &fixture.project,
            checkpoint(
                "final-race",
                "final-race-checkpoint",
                1,
                HostCheckpointKind::PostEditHook,
            ),
        )
        .expect("checkpoint");
    check_calls.store(0, Ordering::SeqCst);
    let barrier = Arc::new(Barrier::new(2));
    let project = fixture.project.clone();
    let revision = checkpointed.session.revision;
    let handles = [(), ()].map(|()| {
        let project = project.clone();
        let barrier = barrier.clone();
        let clock = clock.clone();
        let executor = executor.clone();
        std::thread::spawn(move || {
            barrier.wait();
            RepairHost::with_clock_and_executor(
                FixtureHostAdapter {
                    mode: VerifierMode::Trusted,
                },
                clock,
                executor,
            )
            .finalize(
                &project,
                finalize("final-race", "final-race-same-request", revision),
            )
        })
    });
    let results = handles.map(|handle| handle.join().expect("finalize thread"));
    assert!(results.iter().all(|result| {
        result.is_ok() || matches!(result, Err(RepairHostError::OperationInProgress))
    }));
    assert!(results.iter().any(Result::is_ok));
    assert_eq!(
        check_calls.load(Ordering::SeqCst),
        1,
        "an exact finalization replay must not rerun final verification"
    );
}

#[test]
fn replay_of_ready_feedback_cannot_renew_an_expired_repair_budget() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::AdvanceTo(100),
        },
        clock.clone(),
    );
    let mut repair_context = context();
    repair_context.budget.max_elapsed_seconds = 50;
    let request = begin_request("expired-replay", "expired-replay-begin", repair_context);
    let initial = host
        .begin(&fixture.project, request.clone())
        .expect("ready session");
    assert!(initial.edit_authorized);
    clock.set(151);
    assert!(matches!(
        host.begin(&fixture.project, request),
        Err(RepairHostError::BudgetExhausted)
    ));
}

#[test]
fn begin_replay_rejects_a_changed_authority_expiry() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let clock = FakeClock::new(100);
    let request = begin_request("changed-expiry", "changed-expiry-begin", context());
    RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 1_000,
            completion: CompletionBoundary::AdvanceTo(100),
        },
        clock.clone(),
    )
    .begin(&fixture.project, request.clone())
    .expect("initial grant");

    let error = RepairHost::with_clock(
        BoundaryAdapter {
            clock: clock.clone(),
            expires_at_unix: 2_000,
            completion: CompletionBoundary::AdvanceTo(100),
        },
        clock,
    )
    .begin(&fixture.project, request)
    .expect_err("changed expiry must not replay the original authority");
    assert!(matches!(error, RepairHostError::AuthorityMismatch));
}

#[test]
fn begin_and_checkpoint_never_return_edit_authority_after_reauthentication_expires() {
    for (session, advance_on_call, run_checkpoint) in [
        ("expiry-bootstrap", 2, false),
        ("expiry-checkpoint", 4, true),
    ] {
        let fixture = project("def ReadConfig():\n    pass\n", true, true);
        let clock = FakeClock::new(100);
        let host = RepairHost::with_clock(
            DelayedAuthorityAdapter {
                clock: clock.clone(),
                calls: Arc::new(AtomicU64::new(0)),
                advance_on_call,
            },
            clock,
        );
        let initial = host
            .begin(
                &fixture.project,
                begin_request(session, &format!("{session}-begin"), context()),
            )
            .expect("begin transition");
        if run_checkpoint {
            assert_eq!(initial.state, RepairSessionStateRecord::Ready);
            let result = host
                .checkpoint(
                    &fixture.project,
                    checkpoint(
                        session,
                        &format!("{session}-checkpoint"),
                        initial.session.revision,
                        HostCheckpointKind::PostEditHook,
                    ),
                )
                .expect("expired post-check authority becomes a handoff");
            assert_eq!(result.state, RepairSessionStateRecord::NeedsDecision);
            assert!(!result.edit_authorized);
            assert!(result
                .findings
                .iter()
                .any(|finding| finding.starts_with("authority:")));
        } else {
            assert_eq!(initial.state, RepairSessionStateRecord::NeedsDecision);
            assert!(!initial.edit_authorized);
            assert!(initial
                .findings
                .iter()
                .any(|finding| finding.starts_with("authority:")));
        }
    }
}

#[test]
fn begin_stops_before_reservation_when_authority_expires_during_first_exchange() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let clock = FakeClock::new(100);
    let host = RepairHost::with_clock(
        DelayedAuthorityAdapter {
            clock: clock.clone(),
            calls: Arc::new(AtomicU64::new(0)),
            advance_on_call: 1,
        },
        clock,
    );
    let error = host
        .begin(
            &fixture.project,
            begin_request(
                "first-exchange-expiry",
                "first-exchange-expiry-begin",
                context(),
            ),
        )
        .expect_err("expired first authority exchange must not reserve or check");
    assert!(matches!(error, RepairHostError::AuthorityExpired));
    assert!(DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("private store")
    .all_records()
    .expect("records")
    .is_empty());
}

#[test]
fn a_handoff_reply_is_stale_after_source_changes_or_authority_disappears() {
    let fixture = project("def ReadConfig():\n    pass\n", false, true);
    trusted_host()
        .begin(
            &fixture.project,
            begin_request("stale-handoff", "stale-handoff-begin", context()),
        )
        .expect("handoff session");
    let handoff = current_handoff(&fixture, "stale-handoff");
    fs::write(
        fixture.project.join("src/app.py"),
        "# changed after review\ndef ReadConfig():\n    pass\n",
    )
    .expect("change source");
    assert!(matches!(
        trusted_host().validate_handoff_reply(
            &fixture.project,
            &handoff,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "stale-source-reply",
        ),
        Err(RepairHostError::WorkspaceChanged)
    ));

    let clean = project("def ReadConfig():\n    pass\n", false, true);
    trusted_host()
        .begin(
            &clean.project,
            begin_request("revoked-handoff", "revoked-handoff-begin", context()),
        )
        .expect("handoff session");
    let handoff = current_handoff(&clean, "revoked-handoff");
    assert!(matches!(
        RepairHost::new(FixtureHostAdapter {
            mode: VerifierMode::Missing,
        })
        .validate_handoff_reply(
            &clean.project,
            &handoff,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "revoked-reply",
        ),
        Err(RepairHostError::MissingAuthority)
    ));
}

#[test]
fn handoff_reply_validation_is_observational_and_rechecks_authority_freshness() {
    let fixture = project("def ReadConfig():\n    pass\n", false, true);
    let clock = FakeClock::new(100);
    let calls = Arc::new(AtomicU64::new(0));
    let check_calls = Arc::new(AtomicU64::new(0));
    let host = RepairHost::with_clock_and_executor(
        DelayedAuthorityAdapter {
            clock: clock.clone(),
            calls,
            advance_on_call: 3,
        },
        clock,
        CountingCheckExecutor(check_calls.clone()),
    );
    host.begin(
        &fixture.project,
        begin_request("handoff-observe", "handoff-observe-begin", context()),
    )
    .expect("handoff session");
    check_calls.store(0, Ordering::SeqCst);
    let handoff = current_handoff(&fixture, "handoff-observe");
    let error = host
        .validate_handoff_reply(
            &fixture.project,
            &handoff,
            &RepairAuthorityEvidence {
                locator: "host-grant:task-42".into(),
            },
            "handoff-observe-reply",
        )
        .expect_err("authority expiring during reply validation must fail");
    assert!(matches!(error, RepairHostError::AuthorityExpired));
    assert_eq!(
        check_calls.load(Ordering::SeqCst),
        0,
        "handoff validation must compare stored evidence without spending checker work"
    );
}

#[test]
fn lost_responses_can_be_replayed_without_spending_or_resetting_budget() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let begin = begin_request("replay", "replay-begin", context());
    let first = trusted_host()
        .begin(&fixture.project, begin.clone())
        .expect("begin");
    let replayed = trusted_host()
        .begin(&fixture.project, begin)
        .expect("lost begin response is replayable");
    assert_eq!(replayed, first);
    assert_eq!(session_record(&fixture, "replay").revision, 1);

    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("repair");
    let checkpoint_request = checkpoint(
        "replay",
        "replay-checkpoint",
        1,
        HostCheckpointKind::PostEditHook,
    );
    let checkpointed = trusted_host()
        .checkpoint(&fixture.project, checkpoint_request.clone())
        .expect("checkpoint");
    let mut mismatched_presentation = checkpoint_request.clone();
    mismatched_presentation.kind = HostCheckpointKind::ExplicitCheckpoint;
    assert!(matches!(
        trusted_host().checkpoint(&fixture.project, mismatched_presentation),
        Err(RepairHostError::AuthorityMismatch)
    ));
    let checkpoint_replay = trusted_host()
        .checkpoint(&fixture.project, checkpoint_request)
        .expect("lost checkpoint response is replayable");
    assert_eq!(checkpoint_replay, checkpointed);
    let RecordBody::RepairSession(session) = session_record(&fixture, "replay").body else {
        panic!("session");
    };
    assert_eq!(session.attempts.len(), 1);

    let finalize_request = finalize("replay", "replay-finalize", 2);
    let completed = trusted_host()
        .finalize(&fixture.project, finalize_request.clone())
        .expect("finalize");
    let completion_replay = trusted_host()
        .finalize(&fixture.project, finalize_request)
        .expect("lost finalize response is replayable");
    assert_eq!(completion_replay, completed);
    assert_eq!(session_record(&fixture, "replay").revision, 3);
}

#[test]
fn private_repair_context_cannot_enter_the_shareable_store() {
    let fixture = project("def ReadConfig():\n    pass\n", true, true);
    let feedback = trusted_host()
        .begin(
            &fixture.project,
            begin_request("private-session", "private-session-begin", context()),
        )
        .expect("private repair session");
    let private = DoltRepository::open_existing(
        &fixture.layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("private store");
    let shareable = DoltRepository::initialize(
        &fixture.layout.store_path(StoreKind::Shareable),
        StoreKind::Shareable,
    )
    .expect("shareable store");
    assert!(matches!(
        private.project_selected(&shareable, &[feedback.session], &[]),
        Err(StorageError::InvalidProjectionBoundary)
    ));
    assert!(shareable
        .all_records()
        .expect("shareable records")
        .is_empty());
}
