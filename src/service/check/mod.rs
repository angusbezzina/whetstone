//! `wh check`: run every applicable rule the cheapest reliable way and
//! return actionable findings.
//!
//! Modes: the staged index (pre-commit: mechanical content checks only), the
//! change since a base (pre-push and CI), everything, or a sweep that reports
//! every flag of the rules below must as a finding. Every enforcer is
//! mechanical and runs through `crate::gates`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::domain::{Enforcer, RecordId, RecordRef, Rule, Strength};
use crate::gates::{ArtifactFailure, FileSet, GateOutcome, GateState};
use crate::storage::ProjectLayout;

use super::record::{compute_check_snapshot, digest_bytes};
use super::{
    bounded_request_id, load_records, storage_error, unix_now, unknown_response, utc_now,
    CheckRequest, GateMode, RequiredSnapshot, ServiceEvidence, ServiceResponse, ServiceState,
};

mod findings;
mod report;

pub(crate) use report::{code_tree_digest, Finding, SnapshotBinding};
use report::{CheckReport, CheckResult, CheckState, EvidencePointer};

/// An in-force rule chosen for this check.
pub(crate) struct SelectedRule {
    pub(crate) id: RecordId,
    pub(crate) reference: RecordRef,
    pub(crate) rule: Rule,
}

#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) rules: Vec<SelectedRule>,
    pub(crate) rule_ids: BTreeSet<String>,
    pub(crate) skipped_drafts: Vec<String>,
    pub(crate) changed_paths: Vec<String>,
}

fn invalid(summary: &str) -> ServiceResponse {
    unknown_response("check", "invalid-request".into(), summary.into())
}

pub(super) fn check(mut request: CheckRequest) -> ServiceResponse {
    if request.rules.len() > 64 {
        return invalid("A check accepts at most 64 rule filters.");
    }
    if request
        .rules
        .iter()
        .any(|rule| rule.is_empty() || rule.len() > 256)
        || request
            .timeout_seconds
            .is_some_and(|seconds| seconds == 0 || seconds > 3_600)
    {
        return invalid("A check input exceeds its bound or contains an invalid rule or timeout.");
    }
    // CI sends the all-zero sha for a branch's first push: no base.
    if request
        .base
        .as_deref()
        .is_some_and(|base| !base.is_empty() && base.chars().all(|character| character == '0'))
    {
        request.base = None;
    }
    if request.base.is_some() && request.gate_mode == GateMode::All {
        request.gate_mode = GateMode::Changed;
    }
    let project = match request.project_dir.canonicalize() {
        Ok(project) => project,
        Err(error) => {
            return unknown_response(
                "check",
                request.request_id.unwrap_or_else(|| "check".into()),
                format!("Project path is unavailable: {error}"),
            )
        }
    };
    let request_id = match bounded_request_id(
        request.request_id.clone(),
        format!(
            "check-{:x}",
            Sha256::digest(project.to_string_lossy().as_bytes())
        )[..22]
            .to_string(),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("check", "invalid-request-id".into(), summary),
    };
    let layout = ProjectLayout::resolve(&project).ok();
    // CI enforces what the team accepted and shared, never a contributor's
    // private drafts; a local check sees both stores.
    let agreement = match layout.as_ref().map(load_records) {
        Some(Ok(loaded)) if request.ci => loaded
            .shared
            .clone()
            .filter(|shared| !shared.is_empty())
            .map(AgreementState::from_records),
        Some(Ok(loaded)) if loaded.exists() => Some(AgreementState::from_records(loaded.union().0)),
        Some(Err(error)) => return storage_error("check", request_id, error),
        _ => None,
    };
    if request.ci && agreement.is_none() {
        return unknown_response(
            "check",
            request_id,
            "The CI check enforces the team's shared rules, and this checkout has none; run bd bootstrap, or share accepted rules with wh push.".into(),
        );
    }
    if let Some(pushed) = request.pushed.as_deref() {
        if let Some(reason) = pushed_mismatch(&project, pushed) {
            return unknown_response("check", request_id, reason);
        }
    }
    let files = match request.gate_mode {
        GateMode::Staged => match FileSet::staged(&project) {
            Ok(files) => files,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The staged index could not be read: {error}"),
                )
            }
        },
        _ => FileSet::default(),
    };
    let selection = match select_rules(agreement.as_ref(), &project, &request, &files) {
        Ok(selection) => selection,
        Err(summary) => return unknown_response("check", request_id, summary),
    };
    let files = match request.gate_mode {
        GateMode::Staged => files,
        GateMode::Changed => FileSet::working(&project, &selection.changed_paths),
        _ => FileSet::repository(&project).unwrap_or_default(),
    }
    .retain(|path| !crate::wire::is_generated(path));
    if request.dry_run {
        return dry_run_response(request_id, &request, &selection);
    }
    let snapshot = match compute_check_snapshot(&project, &request.rules) {
        Ok(snapshot) => snapshot,
        Err(error) => return unknown_response("check", request_id, error),
    };
    let mut results = Vec::new();
    // Each check is its own run with its own evidence directory.
    let started_at = format!("{}:{}", utc_now(), std::process::id());
    let run_id = {
        let digest = digest_bytes(
            format!(
                "{request_id}\0{started_at}\0{}\0{:?}",
                selection
                    .rules
                    .iter()
                    .map(|rule| rule.reference.digest.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                request.gate_mode
            )
            .as_bytes(),
        );
        digest.as_str()["sha256:".len().."sha256:".len() + 16].to_string()
    };
    let mut outcomes = Vec::new();
    if let (Some(layout), false) = (layout.as_ref(), selection.rules.is_empty()) {
        let run_dir = crate::gates::evidence_root(layout).join(&run_id);
        if let Err(error) = fs::create_dir_all(&run_dir) {
            return unknown_response(
                "check",
                request_id,
                format!("The evidence directory could not be created: {error}"),
            );
        }
        let timeout = request.timeout_seconds.map_or(
            crate::gates::DEFAULT_GATE_TIMEOUT,
            std::time::Duration::from_secs,
        );
        // What the content gates read: the change without Whetstone's own
        // wiring.
        let content_changed = selection
            .changed_paths
            .iter()
            .filter(|path| !crate::wire::is_generated(path))
            .cloned()
            .collect::<Vec<_>>();
        for selected in &selection.rules {
            let context = crate::gates::GateContext {
                project_root: &project,
                run_dir: &run_dir,
                run_id: &run_id,
                timeout,
                files: &files,
                changed: &content_changed,
            };
            let outcome = crate::gates::run_gate(&context, &selected.id, &selected.rule);
            results.push(gate_result(selected, &outcome, &run_id));
            outcomes.push(outcome);
        }
    }
    let sweep_findings = (request.gate_mode == GateMode::Sweep)
        .then(|| findings::findings(&project, &selection.rules, &outcomes));
    if results.is_empty() {
        let mut response = nothing_ran(request_id, &request, &selection, &outcomes);
        attach_findings(&mut response, sweep_findings.as_deref());
        return response;
    }
    let required = results.iter().any(|result| result.required);
    let report = CheckReport::new(
        format!(
            "project:{:x}:check",
            Sha256::digest(project.to_string_lossy().as_bytes())
        ),
        snapshot,
        results,
        unix_now(),
    );
    let mut state = match report.state {
        CheckState::Success => ServiceState::Success,
        CheckState::Violated => ServiceState::Violated,
        CheckState::Unknown => ServiceState::Unknown,
        CheckState::Unavailable => ServiceState::Unavailable,
    };
    let mut summary = report.human_summary();
    if !required {
        // Only should rules ran: there is no must rule to hold,
        // so the check itself neither passes nor fails a must; its flags are
        // listed below for repair or labelling.
        state = ServiceState::Success;
        summary = "No must rule applies to this check; should results are listed.".into();
    }
    let flags = outcomes
        .iter()
        .filter(|outcome| outcome.state == GateState::Fail && outcome.strength != Strength::Must)
        .map(|outcome| outcome.id.clone())
        .collect::<Vec<_>>();
    if !flags.is_empty() {
        summary.push_str(&format!(
            "\n{} should-rule flag(s) to repair: {}.",
            flags.len(),
            flags.join(", ")
        ));
    }
    let mut response = ServiceResponse::new(request_id, "check", state, summary);
    response.required_snapshot = Some(RequiredSnapshot {
        code_digest: Some(report.snapshot.code_tree.as_str().into()),
        policy_digest: Some(report.snapshot.policy.as_str().into()),
        checker_digest: Some(report.snapshot.checker_bundle.as_str().into()),
        scope_digest: Some(report.snapshot.scope.as_str().into()),
        environment_digest: Some(report.snapshot.environment.as_str().into()),
        trust_digest: Some(report.snapshot.trust.as_str().into()),
    });
    response.evidence = vec![ServiceEvidence {
        kind: "deterministic_scan".into(),
        locator: "local-project".into(),
        digest: Some(report.report_id.as_str().into()),
    }];
    for outcome in &outcomes {
        for evidence in &outcome.evidence {
            response.evidence.push(ServiceEvidence {
                kind: format!("gate:{}", outcome.id),
                locator: evidence.locator.clone(),
                digest: evidence
                    .digest
                    .as_ref()
                    .map(|digest| digest.as_str().to_string()),
            });
        }
    }
    response.permitted_actions = next_actions(state, &outcomes);
    response.data = json!({
        "report": report,
        "gates": outcomes,
        "flags": flags,
        "run_id": if selection.rules.is_empty() { None } else { Some(run_id) },
        "evidence_root": layout.as_ref().map(|layout| crate::gates::evidence_root(layout).display().to_string()),
        "ci": request.ci,
        "selection": selection_json(&request, &selection),
    });
    attach_findings(&mut response, sweep_findings.as_deref());
    response
}

/// One gate run as a report result: a must rule is required, a should rule
/// is not.
fn gate_result(selected: &SelectedRule, outcome: &GateOutcome, run_id: &str) -> CheckResult {
    let id = selected.id.as_str();
    CheckResult {
        requirement_id: format!("gate:{id}"),
        required: selected.rule.strength == Strength::Must,
        state: match outcome.state {
            GateState::Pass => CheckState::Success,
            GateState::Fail => CheckState::Violated,
            GateState::Unknown => CheckState::Unknown,
        },
        summary: outcome.summary.clone(),
        evidence: EvidencePointer {
            source: crate::gates::EVIDENCE_SYSTEM.into(),
            locator: outcome
                .evidence
                .last()
                .map_or_else(|| run_id.to_string(), |evidence| evidence.locator.clone()),
            digest: outcome
                .evidence
                .last()
                .and_then(|evidence| evidence.digest.clone())
                .unwrap_or_else(|| digest_bytes(outcome.summary.as_bytes())),
        },
        findings: outcome
            .failures
            .iter()
            .map(|failure| gate_finding(selected, failure))
            .collect(),
        governing_records: vec![selected.reference.clone()],
        repair_direction:
            "Repair within the current task scope; never change or weaken the rule to pass it."
                .into(),
        permitted_next_action: format!("repair, then wh check --rule {id}"),
        recheck_command: format!("wh check --rule {id}"),
    }
}

fn gate_finding(selected: &SelectedRule, failure: &ArtifactFailure) -> Finding {
    let (file, line) = match failure.location.rsplit_once(':') {
        Some((file, line))
            if line.chars().all(|character| character.is_ascii_digit())
                && !file.contains(' ')
                && !file.starts_with('/')
                && !file.contains("..") =>
        {
            (Some(file.to_string()), line.parse::<u32>().ok())
        }
        _ if !failure.location.contains(' ')
            && !failure.location.starts_with('/')
            && !failure.location.contains("..")
            && failure.location.contains('.') =>
        {
            (Some(failure.location.clone()), None)
        }
        _ => (None, None),
    };
    Finding {
        file,
        line,
        column: None,
        rule_id: selected.id.as_str().into(),
        observed: failure.message.clone(),
        expected: selected.rule.statement.clone(),
        rationale: selected.rule.rationale.clone(),
        repair_direction: "Repair within the current task scope; never weaken the rule.".into(),
        permitted_next_action: format!("repair, then wh check --rule {}", selected.id.as_str()),
        recheck_command: format!("wh check --rule {}", selected.id.as_str()),
    }
}

/// What a sweep adds beside its gates: its findings.
fn attach_findings(
    response: &mut ServiceResponse,
    sweep_findings: Option<&[findings::SweepFinding]>,
) {
    if let Some(found) = sweep_findings {
        response.data["findings"] = json!(found);
        response.data["findings_json"] = json!(findings::json_array(found));
    }
}

fn selection_json(request: &CheckRequest, selection: &Selection) -> Value {
    json!({
        "mode": format!("{:?}", request.gate_mode).to_ascii_lowercase(),
        "rules": selection.rules.iter().map(|rule| rule.id.as_str()).collect::<Vec<_>>(),
        "skipped_drafts": selection.skipped_drafts,
        "changed_paths": selection.changed_paths,
        "base": request.base,
    })
}

fn next_actions(state: ServiceState, outcomes: &[GateOutcome]) -> Vec<String> {
    match state {
        ServiceState::Success => vec!["handoff the verified result".into()],
        ServiceState::Violated => {
            let failing = outcomes
                .iter()
                .filter(|outcome| outcome.state == GateState::Fail)
                .map(|outcome| format!("repair, then wh check --rule {}", outcome.id))
                .collect::<Vec<_>>();
            if failing.is_empty() {
                vec!["repair within the current task scope, then wh check".into()]
            } else {
                failing
            }
        }
        _ => vec!["restore the rule's checker or evidence, then wh check".into()],
    }
}

fn nothing_ran(
    request_id: String,
    request: &CheckRequest,
    selection: &Selection,
    outcomes: &[GateOutcome],
) -> ServiceResponse {
    // Rules are in force, but none runs at pre-commit (command rules run at
    // pre-push): the commit is not blocked on them.
    let deferred = request.gate_mode == GateMode::Staged
        && selection.rules.is_empty()
        && !selection.rule_ids.is_empty()
        && selection.skipped_drafts.is_empty();
    let summary = if deferred {
        "No fast mechanical rule applies at pre-commit; the other rules in force run at pre-push and in CI."
            .to_string()
    } else if selection.skipped_drafts.is_empty() {
        "Nothing ran: no in-force rule applies, which is unknown, not a pass.".to_string()
    } else {
        format!(
            "Nothing ran: {} is a draft and cannot run until accepted; that is unknown, not a pass.",
            selection.skipped_drafts.join(", ")
        )
    };
    let state = if deferred {
        ServiceState::Success
    } else {
        ServiceState::Unknown
    };
    let mut response = ServiceResponse::new(request_id, "check", state, summary);
    response.permitted_actions = if selection.skipped_drafts.is_empty() {
        vec!["record a rule with wh change --kind rule, then accept it".into()]
    } else {
        vec!["accept the draft with wh change --accept <proposal>, then wh check".into()]
    };
    response.data = json!({
        "gates": outcomes,
        "ci": request.ci,
        "selection": selection_json(request, selection),
    });
    response
}

fn dry_run_response(
    request_id: String,
    request: &CheckRequest,
    selection: &Selection,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "check",
        ServiceState::NeedsDecision,
        format!(
            "Dry run: {} rule(s) would run; nothing was executed, asked or recorded.",
            selection.rules.len()
        ),
    );
    response.permitted_actions = vec!["repeat without --dry-run to run them".into()];
    response.data = json!({
        "dry_run": true,
        "gates": selection.rules.iter().map(|selected| {
            let (mechanism, command) = crate::gates::mechanism_label(&selected.rule);
            json!({
                "id": selected.id.as_str(),
                "strength": selected.rule.strength.label(),
                "mechanism": mechanism,
                "command": command,
                "argv": match &selected.rule.enforcer {
                    Enforcer::Test { command } | Enforcer::Validator { command } => {
                        crate::gates::parse_command(command).ok()
                    }
                    _ => None,
                },
                "timeout_seconds": request.timeout_seconds.unwrap_or(crate::gates::DEFAULT_GATE_TIMEOUT.as_secs()),
            })
        }).collect::<Vec<_>>(),
        "environment_allowlist": crate::gates::ENV_ALLOWLIST,
        "selection": selection_json(request, selection),
    });
    response
}

/// Choose the in-force rules this check runs.
pub(crate) fn select_rules(
    agreement: Option<&AgreementState>,
    project: &Path,
    request: &CheckRequest,
    staged: &FileSet,
) -> Result<Selection, String> {
    let mut selection = Selection::default();
    let Some(state) = agreement else {
        return Ok(selection);
    };
    let rule_ids = state.rule_ids();
    for id in &rule_ids {
        selection.rule_ids.insert(id.as_str().to_string());
    }
    let named = request
        .rules
        .iter()
        .filter(|rule| selection.rule_ids.contains(rule.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut wanted: Option<BTreeSet<String>> = (!named.is_empty()).then_some(named);
    let rule_of = |id: &RecordId| -> Option<Rule> {
        state
            .in_force(id)
            .and_then(|record| record.body.rule_view())
            .map(std::borrow::Cow::into_owned)
    };
    match request.gate_mode {
        GateMode::Staged => {
            selection.changed_paths = staged.paths().map(str::to_owned).collect();
            selection
                .changed_paths
                .extend(staged.deleted.iter().cloned());
            let content = rule_ids
                .iter()
                .filter(|id| rule_of(id).is_some_and(|rule| rule.enforcer.runs_staged()))
                .map(|id| id.as_str().to_string())
                .collect::<BTreeSet<_>>();
            wanted = Some(match wanted {
                Some(named) => named.intersection(&content).cloned().collect(),
                None => content,
            });
        }
        GateMode::Changed => {
            let changed = crate::gates::changed_paths_between(
                project,
                request.base.as_deref(),
                request.pushed.is_none(),
            )?;
            selection.changed_paths = changed;
            // Every rule applies to a change.
            let always = rule_ids
                .iter()
                .filter(|id| rule_of(id).is_some())
                .map(|id| id.as_str().to_string())
                .collect::<BTreeSet<_>>();
            if always.is_empty() {
                return Ok(selection);
            }
            wanted = Some(match wanted {
                Some(named) => named,
                None => always,
            });
        }
        GateMode::All | GateMode::Sweep => {}
    }
    for id in &rule_ids {
        if wanted
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(id.as_str()))
        {
            continue;
        }
        let Some(record) = state.in_force(id) else {
            selection.skipped_drafts.push(id.as_str().to_string());
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        if request.gate_mode == GateMode::Sweep && !findings::sweepable(&rule) {
            continue;
        }
        let reference = record.reference().map_err(|error| error.to_string())?;
        selection.rules.push(SelectedRule {
            id: id.clone(),
            reference,
            rule: rule.into_owned(),
        });
    }
    if request.gate_mode != GateMode::Staged {
        if let Some(wanted) = wanted {
            for name in wanted {
                if !selection.rule_ids.contains(name.as_str()) {
                    return Err(format!("{name} is not a rule in this agreement."));
                }
            }
        }
    }
    Ok(selection)
}

/// Why the working tree is not exactly the commit being pushed, if it is
/// not. The check runs commands and reads files in the working tree, so it
/// can only vouch for a push when that tree is the pushed commit.
fn pushed_mismatch(project: &Path, pushed: &str) -> Option<String> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(project)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    if pushed.len() < 7
        || !pushed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Some(format!(
            "{pushed} is not a commit id; the pre-push hook passes the pushed commit."
        ));
    }
    let head = git(&["rev-parse", "--verify", "--quiet", "HEAD"])?;
    let pushed_commit = git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &format!("{pushed}^{{commit}}"),
    ]);
    if pushed_commit.as_deref() != Some(head.as_str()) {
        return Some(format!(
            "The push sends {} but {} is checked out; the check reads the working tree, so it cannot vouch for a commit that is not checked out. Check out the branch you push (or push the checked-out one), then push again.",
            &pushed[..pushed.len().min(12)],
            &head[..head.len().min(12)]
        ));
    }
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    if !dirty.is_empty() {
        return Some(
            "Tracked files have uncommitted changes, so the working tree is not what you push. Commit or stash them, then push again."
                .into(),
        );
    }
    None
}
