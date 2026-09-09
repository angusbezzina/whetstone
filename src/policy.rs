//! Deterministic policy applicability and compatibility resolution.
//!
//! This module resolves approved standards from declared facts. A skill may
//! supply a semantic relevance hypothesis, but that hypothesis can never omit
//! a required team standard or change the required snapshot.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    AgreementRecord, ContentDigest, ProvenanceAuthority, RecordBody, RecordId, RecordRef, Scope,
    StandardStrength,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyClass {
    Team,
    Personal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    Current,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Applicability {
    #[serde(default)]
    pub path_prefixes: BTreeSet<PathBuf>,
    #[serde(default)]
    pub components: BTreeSet<String>,
    #[serde(default)]
    pub operations: BTreeSet<String>,
    #[serde(default)]
    pub environments: BTreeSet<String>,
    #[serde(default)]
    pub semantic_intents: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericDirection {
    Maximum,
    Minimum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "constraint_type", rename_all = "snake_case")]
pub enum Constraint {
    Numeric {
        metric: String,
        unit: String,
        window: String,
        direction: NumericDirection,
        threshold: i64,
    },
    Choice {
        dimension: String,
        value: String,
    },
    Semantic {
        topic: String,
    },
}

impl Constraint {
    fn validate(&self) -> Result<(), PolicyError> {
        let fields: Vec<&str> = match self {
            Self::Numeric {
                metric,
                unit,
                window,
                ..
            } => vec![metric, unit, window],
            Self::Choice { dimension, value } => vec![dimension, value],
            Self::Semantic { topic } => vec![topic],
        };
        if fields.iter().any(|value| value.trim().is_empty()) {
            Err(PolicyError::InvalidInput(
                "constraint fields must not be empty".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn family(&self) -> (&str, &str) {
        match self {
            Self::Numeric { metric, .. } => ("numeric", metric),
            Self::Choice { dimension, .. } => ("choice", dimension),
            Self::Semantic { topic } => ("semantic", topic),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyCandidate {
    pub record: AgreementRecord,
    pub class: PolicyClass,
    pub evidence_state: EvidenceState,
    pub applicability: Applicability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Constraint>,
    #[serde(default)]
    pub governing_records: Vec<RecordRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizedException {
    pub decision: RecordRef,
    pub parent_standard: RecordRef,
    pub child_standard: RecordRef,
    pub scope: Scope,
    pub authority: ProvenanceAuthority,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RelevanceHypothesis {
    #[serde(default)]
    pub semantic_intents: BTreeSet<String>,
    #[serde(default)]
    pub relevant_standard_ids: BTreeSet<RecordId>,
    #[serde(default)]
    pub irrelevant_standard_ids: BTreeSet<RecordId>,
    #[serde(default)]
    pub missing_context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRequest {
    pub project_root: PathBuf,
    pub accepted_scope: Scope,
    #[serde(default)]
    pub declared_paths: Vec<PathBuf>,
    #[serde(default)]
    pub components: BTreeSet<String>,
    #[serde(default)]
    pub component_dependencies: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub operations: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default)]
    pub relevance_hypothesis: RelevanceHypothesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Ready,
    ReviewNeeded,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicabilityState {
    Applies,
    DoesNotApply,
    JudgmentRequired,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStandard {
    pub standard: RecordRef,
    pub scope: Scope,
    pub source_id: RecordId,
    pub evidence_state: EvidenceState,
    pub applicability: ApplicabilityState,
    pub rationale: Vec<String>,
    pub governing_records: Vec<RecordRef>,
    pub boundary: ResolutionBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionBoundary {
    Deterministic,
    JudgmentOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalDisposition {
    CompatibleNarrowing,
    CompatibleIndependent,
    Conflict,
    ReviewNeeded,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalResolution {
    pub standard: RecordRef,
    pub disposition: PersonalDisposition,
    pub compared_with: Vec<RecordRef>,
    pub rationale: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConflict {
    pub team_standard: RecordRef,
    pub other_standard: RecordRef,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_exception: Option<RecordRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionTrace {
    pub standard: RecordRef,
    pub class: PolicyClass,
    pub applicability: ApplicabilityState,
    pub facts_used: Vec<String>,
    pub facts_missing: Vec<String>,
    pub hypothesis_observed: bool,
    pub hypothesis_effect: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyResolution {
    pub schema_version: u16,
    pub status: ResolutionStatus,
    pub required_snapshot: ContentDigest,
    pub required: Vec<ResolvedStandard>,
    pub personal: Vec<PersonalResolution>,
    pub conflicts: Vec<PolicyConflict>,
    pub trace: Vec<ResolutionTrace>,
    pub missing_context: Vec<String>,
}

pub struct PolicyResolver;

impl PolicyResolver {
    pub fn resolve(
        request: &ResolutionRequest,
        candidates: &[PolicyCandidate],
        exceptions: &[AuthorizedException],
    ) -> Result<PolicyResolution, PolicyError> {
        request
            .accepted_scope
            .validate()
            .map_err(PolicyError::Domain)?;
        let facts = DeclaredFacts::build(request)?;
        let mut candidates = candidates.to_vec();
        candidates.sort_by(candidate_order);
        validate_candidates(&candidates)?;
        validate_exceptions(exceptions)?;

        let mut required = Vec::new();
        let mut personal = Vec::new();
        let mut conflicts = Vec::new();
        let mut trace = Vec::new();
        let mut missing_context = request.relevance_hypothesis.missing_context.clone();
        missing_context.extend(facts.missing_context.clone());

        let mut applicable_team = Vec::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.class == PolicyClass::Team)
        {
            let evaluation = evaluate_applicability(candidate, request, &facts);
            let standard = standard(candidate)?;
            let is_mandatory = standard.strength == StandardStrength::Must;
            let in_required_set =
                is_mandatory && evaluation.state != ApplicabilityState::DoesNotApply;

            if evaluation.state != ApplicabilityState::DoesNotApply {
                applicable_team.push(candidate);
            }
            if in_required_set {
                required.push(ResolvedStandard {
                    standard: candidate.record.reference().map_err(PolicyError::Domain)?,
                    scope: candidate.record.scope.clone(),
                    source_id: candidate.record.id.clone(),
                    evidence_state: candidate.evidence_state,
                    applicability: evaluation.state,
                    rationale: resolution_rationale(candidate, &evaluation),
                    governing_records: sorted_refs(&candidate.governing_records),
                    boundary: if evaluation.state == ApplicabilityState::JudgmentRequired {
                        ResolutionBoundary::JudgmentOnly
                    } else {
                        ResolutionBoundary::Deterministic
                    },
                });
            }
            trace.push(trace_for(candidate, request, evaluation));
        }

        conflicts.extend(resolve_child_scope_relaxations(
            &applicable_team,
            &request.accepted_scope,
            exceptions,
        )?);

        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.class == PolicyClass::Personal)
        {
            let evaluation = evaluate_applicability(candidate, request, &facts);
            trace.push(trace_for(candidate, request, evaluation.clone()));
            if evaluation.state == ApplicabilityState::DoesNotApply {
                personal.push(personal_result(
                    candidate,
                    PersonalDisposition::NotApplicable,
                    &[],
                    "personal constraint is outside the declared scope",
                )?);
                continue;
            }
            if evaluation.state != ApplicabilityState::Applies
                || candidate.evidence_state != EvidenceState::Current
            {
                personal.push(personal_result(
                    candidate,
                    PersonalDisposition::ReviewNeeded,
                    &[],
                    "personal constraint needs current deterministic context",
                )?);
                continue;
            }
            let relevant_team: Vec<&PolicyCandidate> = applicable_team
                .iter()
                .copied()
                .filter(|team| constraints_overlap(team, candidate))
                .collect();
            if relevant_team.is_empty() {
                personal.push(personal_result(
                    candidate,
                    PersonalDisposition::CompatibleIndependent,
                    &[],
                    "personal constraint does not replace a team requirement",
                )?);
                continue;
            }

            let mut disposition = PersonalDisposition::CompatibleNarrowing;
            let mut reasons = Vec::new();
            let mut compared = Vec::new();
            for team in relevant_team {
                let team_ref = team.record.reference().map_err(PolicyError::Domain)?;
                let personal_ref = candidate.record.reference().map_err(PolicyError::Domain)?;
                compared.push(team_ref.clone());
                match compare_constraints(team.constraint.as_ref(), candidate.constraint.as_ref()) {
                    Compatibility::Narrower(reason) => reasons.push(reason),
                    Compatibility::Conflict(reason) => {
                        disposition = PersonalDisposition::Conflict;
                        reasons.push(reason.clone());
                        conflicts.push(PolicyConflict {
                            team_standard: team_ref,
                            other_standard: personal_ref.clone(),
                            reason,
                            authorized_exception: None,
                        });
                    }
                    Compatibility::Review(reason) => {
                        if disposition != PersonalDisposition::Conflict {
                            disposition = PersonalDisposition::ReviewNeeded;
                        }
                        reasons.push(reason);
                    }
                }
            }
            compared.sort();
            compared.dedup();
            personal.push(PersonalResolution {
                standard: candidate.record.reference().map_err(PolicyError::Domain)?,
                disposition,
                compared_with: compared,
                rationale: reasons,
            });
        }

        required.sort_by(|left, right| left.standard.cmp(&right.standard));
        personal.sort_by(|left, right| left.standard.cmp(&right.standard));
        conflicts.sort_by(|left, right| {
            (&left.team_standard, &left.other_standard, &left.reason).cmp(&(
                &right.team_standard,
                &right.other_standard,
                &right.reason,
            ))
        });
        trace.sort_by(|left, right| left.standard.cmp(&right.standard));
        missing_context.sort();
        missing_context.dedup();

        let required_snapshot = digest_required(&required)?;
        let status = if !conflicts.is_empty() {
            ResolutionStatus::Conflict
        } else if required.iter().any(|standard| {
            standard.evidence_state == EvidenceState::Unknown
                || standard.applicability == ApplicabilityState::Unknown
        }) {
            ResolutionStatus::Unknown
        } else if required.iter().any(|standard| {
            standard.evidence_state == EvidenceState::Stale
                || standard.applicability == ApplicabilityState::JudgmentRequired
        }) || personal
            .iter()
            .any(|result| result.disposition == PersonalDisposition::ReviewNeeded)
        {
            ResolutionStatus::ReviewNeeded
        } else {
            ResolutionStatus::Ready
        };

        Ok(PolicyResolution {
            schema_version: 1,
            status,
            required_snapshot,
            required,
            personal,
            conflicts,
            trace,
            missing_context,
        })
    }
}

#[derive(Debug, Clone)]
struct ApplicabilityEvaluation {
    state: ApplicabilityState,
    facts_used: Vec<String>,
    facts_missing: Vec<String>,
}

#[derive(Debug)]
struct DeclaredFacts {
    paths: Vec<PathBuf>,
    components: BTreeSet<String>,
    missing_context: Vec<String>,
}

impl DeclaredFacts {
    fn build(request: &ResolutionRequest) -> Result<Self, PolicyError> {
        let root = request
            .project_root
            .canonicalize()
            .map_err(|error| PolicyError::Path {
                path: request.project_root.clone(),
                reason: error.to_string(),
            })?;
        if !root.is_dir() {
            return Err(PolicyError::Path {
                path: root,
                reason: "project root is not a directory".into(),
            });
        }
        let mut paths = Vec::new();
        let mut missing_context = Vec::new();
        for declared in &request.declared_paths {
            let relative = safe_relative_path(declared)?;
            let joined = root.join(&relative);
            match joined.canonicalize() {
                Ok(canonical) => {
                    if !canonical.starts_with(&root) {
                        return Err(PolicyError::PathEscape(declared.clone()));
                    }
                    let canonical_relative = canonical
                        .strip_prefix(&root)
                        .map_err(|_| PolicyError::PathEscape(declared.clone()))?
                        .to_path_buf();
                    paths.push(canonical_relative);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ensure_existing_ancestor_inside(&root, &joined, declared)?;
                    missing_context.push(format!(
                        "declared path does not exist: {}",
                        relative.display()
                    ));
                    paths.push(relative);
                }
                Err(error) => {
                    return Err(PolicyError::Path {
                        path: declared.clone(),
                        reason: error.to_string(),
                    });
                }
            }
        }
        paths.sort();
        paths.dedup();

        let mut components = request.components.clone();
        let mut queue: VecDeque<String> = components.iter().cloned().collect();
        while let Some(component) = queue.pop_front() {
            if let Some(dependencies) = request.component_dependencies.get(&component) {
                for dependency in dependencies {
                    validate_fact("component dependency", dependency)?;
                    if components.insert(dependency.clone()) {
                        queue.push_back(dependency.clone());
                    }
                }
            }
        }
        for component in request.component_dependencies.keys() {
            validate_fact("component", component)?;
        }
        for component in &request.components {
            validate_fact("component", component)?;
        }
        for operation in &request.operations {
            validate_fact("operation", operation)?;
        }
        if let Some(environment) = &request.environment {
            validate_fact("environment", environment)?;
        }
        Ok(Self {
            paths,
            components,
            missing_context,
        })
    }
}

fn safe_relative_path(path: &Path) -> Result<PathBuf, PolicyError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(PolicyError::UnsafePath(path.to_path_buf()));
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PolicyError::UnsafePath(path.to_path_buf()));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        Err(PolicyError::UnsafePath(path.to_path_buf()))
    } else {
        Ok(safe)
    }
}

fn ensure_existing_ancestor_inside(
    root: &Path,
    joined: &Path,
    declared: &Path,
) -> Result<(), PolicyError> {
    let mut ancestor = joined.parent();
    while let Some(path) = ancestor {
        match fs::canonicalize(path) {
            Ok(canonical) => {
                if canonical.starts_with(root) {
                    return Ok(());
                }
                return Err(PolicyError::PathEscape(declared.to_path_buf()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = path.parent();
            }
            Err(error) => {
                return Err(PolicyError::Path {
                    path: declared.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        }
    }
    Err(PolicyError::PathEscape(declared.to_path_buf()))
}

fn validate_candidates(candidates: &[PolicyCandidate]) -> Result<(), PolicyError> {
    for candidate in candidates {
        candidate.record.validate().map_err(PolicyError::Domain)?;
        standard(candidate)?;
        for prefix in &candidate.applicability.path_prefixes {
            safe_relative_path(prefix)?;
        }
        for fact in candidate
            .applicability
            .components
            .iter()
            .chain(candidate.applicability.operations.iter())
            .chain(candidate.applicability.environments.iter())
            .chain(candidate.applicability.semantic_intents.iter())
        {
            validate_fact("applicability", fact)?;
        }
        if let Some(constraint) = &candidate.constraint {
            constraint.validate()?;
        }
        for governing in &candidate.governing_records {
            if governing.revision == 0 {
                return Err(PolicyError::InvalidInput(
                    "governing record revision must be positive".into(),
                ));
            }
        }
        match candidate.class {
            PolicyClass::Team
                if candidate.record.provenance.authority
                    != ProvenanceAuthority::IndependentlyApproved =>
            {
                return Err(PolicyError::UntrustedTeamStandard(
                    candidate.record.id.clone(),
                ));
            }
            PolicyClass::Personal
                if candidate.record.provenance.authority
                    == ProvenanceAuthority::IndependentlyApproved =>
            {
                return Err(PolicyError::InvalidInput(format!(
                    "personal standard {} cannot masquerade as team-approved",
                    candidate.record.id.as_str()
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_exceptions(exceptions: &[AuthorizedException]) -> Result<(), PolicyError> {
    for exception in exceptions {
        exception.scope.validate().map_err(PolicyError::Domain)?;
        if exception.authority != ProvenanceAuthority::IndependentlyApproved {
            return Err(PolicyError::UnauthorizedException(
                exception.decision.clone(),
            ));
        }
        if exception.rationale.trim().is_empty() {
            return Err(PolicyError::InvalidInput(
                "authorized exception requires a rationale".into(),
            ));
        }
    }
    Ok(())
}

fn standard(candidate: &PolicyCandidate) -> Result<&crate::domain::Standard, PolicyError> {
    match &candidate.record.body {
        RecordBody::Standard(standard) => Ok(standard),
        _ => Err(PolicyError::NotAStandard(candidate.record.id.clone())),
    }
}

fn evaluate_applicability(
    candidate: &PolicyCandidate,
    request: &ResolutionRequest,
    facts: &DeclaredFacts,
) -> ApplicabilityEvaluation {
    let mut used = vec![format!("scope:{}", scope_label(&candidate.record.scope))];
    let mut missing = Vec::new();
    if !candidate.record.scope.contains(&request.accepted_scope) {
        return ApplicabilityEvaluation {
            state: ApplicabilityState::DoesNotApply,
            facts_used: used,
            facts_missing: missing,
        };
    }

    let applicability = &candidate.applicability;
    let deterministic_dimensions = !applicability.path_prefixes.is_empty()
        || !applicability.components.is_empty()
        || !applicability.operations.is_empty()
        || !applicability.environments.is_empty();
    let path_match = applicability.path_prefixes.is_empty()
        || facts.paths.iter().any(|path| {
            applicability
                .path_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
        });
    let component_match = applicability.components.is_empty()
        || !applicability.components.is_disjoint(&facts.components);
    let operation_match = applicability.operations.is_empty()
        || !applicability.operations.is_disjoint(&request.operations);
    let environment_match = applicability.environments.is_empty()
        || request
            .environment
            .as_ref()
            .is_some_and(|environment| applicability.environments.contains(environment));

    if !applicability.path_prefixes.is_empty() {
        used.push(format!(
            "declared_paths:{}",
            facts
                .paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
        if facts.paths.is_empty() {
            missing.push("declared paths were not supplied".into());
        }
    }
    if !applicability.components.is_empty() {
        used.push(format!(
            "component_closure:{}",
            facts
                .components
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ));
        if facts.components.is_empty() {
            missing.push("components were not supplied".into());
        }
    }
    if !applicability.operations.is_empty() {
        used.push(format!(
            "operations:{}",
            request
                .operations
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ));
        if request.operations.is_empty() {
            missing.push("operations were not supplied".into());
        }
    }
    if !applicability.environments.is_empty() {
        used.push(format!(
            "environment:{}",
            request.environment.as_deref().unwrap_or("<missing>")
        ));
        if request.environment.is_none() {
            missing.push("environment was not supplied".into());
        }
    }

    if (!applicability.path_prefixes.is_empty() && facts.paths.is_empty())
        || (!applicability.components.is_empty() && facts.components.is_empty())
        || (!applicability.operations.is_empty() && request.operations.is_empty())
        || (!applicability.environments.is_empty() && request.environment.is_none())
    {
        return ApplicabilityEvaluation {
            state: ApplicabilityState::Unknown,
            facts_used: used,
            facts_missing: missing,
        };
    }
    if deterministic_dimensions
        && !(path_match && component_match && operation_match && environment_match)
    {
        return ApplicabilityEvaluation {
            state: ApplicabilityState::DoesNotApply,
            facts_used: used,
            facts_missing: missing,
        };
    }
    if !applicability.semantic_intents.is_empty() {
        used.push(format!(
            "skill_hypothesis:{}",
            request
                .relevance_hypothesis
                .semantic_intents
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ));
        return ApplicabilityEvaluation {
            state: ApplicabilityState::JudgmentRequired,
            facts_used: used,
            facts_missing: missing,
        };
    }
    ApplicabilityEvaluation {
        state: ApplicabilityState::Applies,
        facts_used: used,
        facts_missing: missing,
    }
}

fn trace_for(
    candidate: &PolicyCandidate,
    request: &ResolutionRequest,
    evaluation: ApplicabilityEvaluation,
) -> ResolutionTrace {
    let hypothesis_observed = request
        .relevance_hypothesis
        .relevant_standard_ids
        .contains(&candidate.record.id)
        || request
            .relevance_hypothesis
            .irrelevant_standard_ids
            .contains(&candidate.record.id)
        || !candidate.applicability.semantic_intents.is_empty();
    ResolutionTrace {
        standard: candidate
            .record
            .reference()
            .expect("validated candidates always produce references"),
        class: candidate.class,
        applicability: evaluation.state,
        facts_used: evaluation.facts_used,
        facts_missing: evaluation.facts_missing,
        hypothesis_observed,
        hypothesis_effect: if hypothesis_observed {
            "informational only; it cannot add or remove a deterministic team requirement".into()
        } else {
            "none".into()
        },
    }
}

fn resolution_rationale(
    candidate: &PolicyCandidate,
    evaluation: &ApplicabilityEvaluation,
) -> Vec<String> {
    let mut rationale = vec![match evaluation.state {
        ApplicabilityState::Applies => {
            "approved mandatory team standard matches declared deterministic facts".into()
        }
        ApplicabilityState::JudgmentRequired => {
            "approved mandatory team standard requires accountable semantic review".into()
        }
        ApplicabilityState::Unknown => {
            "approved mandatory team standard remains required because applicability facts are missing"
                .into()
        }
        ApplicabilityState::DoesNotApply => "outside declared applicability".into(),
    }];
    if !candidate.governing_records.is_empty() {
        rationale.push("linked to governing goal or decision records".into());
    } else {
        rationale.push("no governing goal or decision link was supplied".into());
    }
    if candidate.evidence_state != EvidenceState::Current {
        rationale.push(format!(
            "source evidence is {} and cannot yield a green result",
            match candidate.evidence_state {
                EvidenceState::Current => "current",
                EvidenceState::Stale => "stale",
                EvidenceState::Unknown => "unknown",
            }
        ));
    }
    rationale
}

fn resolve_child_scope_relaxations(
    team: &[&PolicyCandidate],
    accepted_scope: &Scope,
    exceptions: &[AuthorizedException],
) -> Result<Vec<PolicyConflict>, PolicyError> {
    let mut conflicts = Vec::new();
    for parent in team {
        for child in team {
            if parent.record.id == child.record.id
                || parent.record.scope == child.record.scope
                || !parent.record.scope.contains(&child.record.scope)
                || !child.record.scope.contains(accepted_scope)
                || !constraints_overlap(parent, child)
            {
                continue;
            }
            let comparison =
                compare_constraints(parent.constraint.as_ref(), child.constraint.as_ref());
            let reason = match comparison {
                Compatibility::Narrower(_) => continue,
                Compatibility::Conflict(reason) | Compatibility::Review(reason) => reason,
            };
            let parent_ref = parent.record.reference().map_err(PolicyError::Domain)?;
            let child_ref = child.record.reference().map_err(PolicyError::Domain)?;
            let exception = exceptions.iter().find(|exception| {
                exception.parent_standard == parent_ref
                    && exception.child_standard == child_ref
                    && exception.scope.contains(accepted_scope)
            });
            if exception.is_none() {
                conflicts.push(PolicyConflict {
                    team_standard: parent_ref,
                    other_standard: child_ref,
                    reason: format!(
                        "child scope relaxes or changes its parent without an authorized exception: {reason}"
                    ),
                    authorized_exception: None,
                });
            }
        }
    }
    Ok(conflicts)
}

fn constraints_overlap(left: &PolicyCandidate, right: &PolicyCandidate) -> bool {
    match (&left.constraint, &right.constraint) {
        (Some(left), Some(right)) => left.family() == right.family(),
        _ => false,
    }
}

enum Compatibility {
    Narrower(String),
    Conflict(String),
    Review(String),
}

fn compare_constraints(team: Option<&Constraint>, other: Option<&Constraint>) -> Compatibility {
    match (team, other) {
        (
            Some(Constraint::Numeric {
                metric: team_metric,
                unit: team_unit,
                window: team_window,
                direction: team_direction,
                threshold: team_threshold,
            }),
            Some(Constraint::Numeric {
                metric,
                unit,
                window,
                direction,
                threshold,
            }),
        ) => {
            if team_metric != metric || team_unit != unit || team_window != window {
                return Compatibility::Conflict(
                    "numeric constraints cannot be compared unless metric, unit, and observation window are identical"
                        .into(),
                );
            }
            if team_direction != direction {
                return Compatibility::Conflict(
                    "numeric constraints use incompatible directions".into(),
                );
            }
            let narrower = match direction {
                NumericDirection::Maximum => threshold <= team_threshold,
                NumericDirection::Minimum => threshold >= team_threshold,
            };
            if narrower {
                Compatibility::Narrower(format!(
                    "personal threshold {threshold} is no weaker than team threshold {team_threshold}"
                ))
            } else {
                Compatibility::Conflict(format!(
                    "personal threshold {threshold} weakens team threshold {team_threshold}"
                ))
            }
        }
        (
            Some(Constraint::Choice {
                dimension: team_dimension,
                value: team_value,
            }),
            Some(Constraint::Choice { dimension, value }),
        ) if team_dimension == dimension => {
            if team_value == value {
                Compatibility::Narrower("choice matches the team requirement".into())
            } else {
                Compatibility::Conflict(format!(
                    "{dimension} choices conflict ({team_value} versus {value}); neither is inherently stricter"
                ))
            }
        }
        (Some(Constraint::Semantic { .. }), Some(Constraint::Semantic { .. })) => {
            Compatibility::Review(
                "semantic constraints need accountable review; deterministic compatibility is unknown"
                    .into(),
            )
        }
        (Some(_), Some(_)) => Compatibility::Conflict(
            "constraints describe incompatible definitions and cannot override one another".into(),
        ),
        _ => Compatibility::Review(
            "unstructured compatibility cannot be determined without judgment".into(),
        ),
    }
}

fn personal_result(
    candidate: &PolicyCandidate,
    disposition: PersonalDisposition,
    compared: &[RecordRef],
    reason: &str,
) -> Result<PersonalResolution, PolicyError> {
    Ok(PersonalResolution {
        standard: candidate.record.reference().map_err(PolicyError::Domain)?,
        disposition,
        compared_with: sorted_refs(compared),
        rationale: vec![reason.into()],
    })
}

fn candidate_order(left: &PolicyCandidate, right: &PolicyCandidate) -> std::cmp::Ordering {
    (&left.record.id, left.record.revision, left.class).cmp(&(
        &right.record.id,
        right.record.revision,
        right.class,
    ))
}

fn sorted_refs(references: &[RecordRef]) -> Vec<RecordRef> {
    let mut references = references.to_vec();
    references.sort();
    references.dedup();
    references
}

fn digest_required(required: &[ResolvedStandard]) -> Result<ContentDigest, PolicyError> {
    let bytes = serde_json::to_vec(required)
        .map_err(|error| PolicyError::Serialization(error.to_string()))?;
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))).map_err(PolicyError::Domain)
}

fn validate_fact(label: &str, value: &str) -> Result<(), PolicyError> {
    if value.trim().is_empty() || value.len() > 160 {
        Err(PolicyError::InvalidInput(format!(
            "{label} must contain 1 to 160 non-whitespace characters"
        )))
    } else {
        Ok(())
    }
}

fn scope_label(scope: &Scope) -> String {
    format!(
        "{}/{}/{}/{}",
        scope.organization.as_deref().unwrap_or("*"),
        scope.project,
        scope.component.as_deref().unwrap_or("*"),
        scope.environment.as_deref().unwrap_or("*")
    )
}

#[derive(Debug)]
pub enum PolicyError {
    Domain(crate::domain::DomainError),
    InvalidInput(String),
    UnsafePath(PathBuf),
    PathEscape(PathBuf),
    Path { path: PathBuf, reason: String },
    NotAStandard(RecordId),
    UntrustedTeamStandard(RecordId),
    UnauthorizedException(RecordRef),
    Serialization(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(error) => write!(formatter, "invalid agreement record: {error}"),
            Self::InvalidInput(message) => write!(formatter, "invalid policy input: {message}"),
            Self::UnsafePath(path) => write!(formatter, "unsafe declared path: {}", path.display()),
            Self::PathEscape(path) => write!(
                formatter,
                "declared path escapes the canonical project root: {}",
                path.display()
            ),
            Self::Path { path, reason } => {
                write!(
                    formatter,
                    "cannot resolve path {}: {reason}",
                    path.display()
                )
            }
            Self::NotAStandard(id) => {
                write!(formatter, "policy candidate {id:?} is not a standard")
            }
            Self::UntrustedTeamStandard(id) => {
                write!(formatter, "team standard {id:?} lacks independent approval")
            }
            Self::UnauthorizedException(reference) => write!(
                formatter,
                "exception {}@{} lacks independent approval",
                reference.id.as_str(),
                reference.revision
            ),
            Self::Serialization(error) => write!(formatter, "policy serialization failed: {error}"),
        }
    }
}

impl std::error::Error for PolicyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Enforcement, EvidenceRef, PrincipalKind, PrincipalRef, Provenance, ProvenanceKind,
        Standard, SCHEMA_VERSION_V1,
    };
    use tempfile::TempDir;

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::new(format!("sha256:{}", byte.to_string().repeat(64)))
            .expect("test digest is valid")
    }

    fn reference(id: &str) -> RecordRef {
        RecordRef {
            id: RecordId::new(id).expect("test ID is valid"),
            revision: 1,
            digest: digest('a'),
        }
    }

    fn scope(component: Option<&str>, environment: Option<&str>) -> Scope {
        Scope {
            organization: Some("acme".into()),
            project: "anvil".into(),
            component: component.map(str::to_owned),
            environment: environment.map(str::to_owned),
        }
    }

    fn candidate(
        id: &str,
        class: PolicyClass,
        record_scope: Scope,
        applicability: Applicability,
        constraint: Option<Constraint>,
    ) -> PolicyCandidate {
        let authority = match class {
            PolicyClass::Team => ProvenanceAuthority::IndependentlyApproved,
            PolicyClass::Personal => ProvenanceAuthority::OwnerAuthored,
        };
        PolicyCandidate {
            record: AgreementRecord {
                schema_version: SCHEMA_VERSION_V1,
                id: RecordId::new(id).expect("test ID is valid"),
                revision: 1,
                scope: record_scope,
                owner: PrincipalRef {
                    kind: PrincipalKind::LocalUser,
                    stable_id: "owner-1".into(),
                    display_name: None,
                },
                provenance: Provenance {
                    kind: ProvenanceKind::HumanAuthored,
                    recorded_by: PrincipalRef {
                        kind: PrincipalKind::LocalUser,
                        stable_id: "author-1".into(),
                        display_name: None,
                    },
                    recorded_at: "2026-09-09T00:00:00Z".into(),
                    sources: vec![EvidenceRef {
                        system: "decision-log".into(),
                        locator: format!("decision:{id}"),
                        digest: Some(digest('b')),
                    }],
                    authority,
                },
                supersedes: None,
                idempotency_key: format!("create:{id}"),
                body: RecordBody::Standard(Standard {
                    statement: format!("enforce {id}"),
                    rationale: "accepted engineering policy".into(),
                    strength: StandardStrength::Must,
                    enforcement: Enforcement::Test {
                        command_ref: format!("check:{id}"),
                    },
                    examples: Vec::new(),
                }),
            },
            class,
            evidence_state: EvidenceState::Current,
            applicability,
            constraint,
            governing_records: vec![reference("decision:goal")],
        }
    }

    fn request(root: &Path) -> ResolutionRequest {
        ResolutionRequest {
            project_root: root.to_path_buf(),
            accepted_scope: scope(None, None),
            declared_paths: Vec::new(),
            components: BTreeSet::new(),
            component_dependencies: BTreeMap::new(),
            operations: BTreeSet::new(),
            environment: None,
            relevance_hypothesis: RelevanceHypothesis::default(),
        }
    }

    fn numeric(metric: &str, unit: &str, window: &str, threshold: i64) -> Constraint {
        Constraint::Numeric {
            metric: metric.into(),
            unit: unit.into(),
            window: window.into(),
            direction: NumericDirection::Maximum,
            threshold,
        }
    }

    #[test]
    fn required_snapshot_is_deterministic_and_agent_omission_cannot_remove_rules() {
        let root = TempDir::new().expect("temp project");
        let alpha = candidate(
            "standard:alpha",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            None,
        );
        let beta = candidate(
            "standard:beta",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            None,
        );
        let base = request(root.path());
        let first = PolicyResolver::resolve(&base, &[beta.clone(), alpha.clone()], &[])
            .expect("facts resolve");

        let mut hostile_hypothesis = base.clone();
        hostile_hypothesis
            .relevance_hypothesis
            .irrelevant_standard_ids
            .extend([alpha.record.id.clone(), beta.record.id.clone()]);
        let second = PolicyResolver::resolve(&hostile_hypothesis, &[alpha, beta], &[])
            .expect("hypothesis is informational");

        assert_eq!(first.required_snapshot, second.required_snapshot);
        assert_eq!(first.required, second.required);
        assert_eq!(first.required.len(), 2);
        assert!(second.trace.iter().all(|entry| {
            entry.hypothesis_effect.contains("cannot add or remove")
                && entry.applicability == ApplicabilityState::Applies
        }));
    }

    #[test]
    fn numeric_narrowing_requires_identical_metric_unit_and_window() {
        let root = TempDir::new().expect("temp project");
        let team = candidate(
            "standard:latency-team",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            Some(numeric("p95-latency", "ms", "5m", 100)),
        );
        let narrower = candidate(
            "standard:latency-local",
            PolicyClass::Personal,
            scope(None, None),
            Applicability::default(),
            Some(numeric("p95-latency", "ms", "5m", 80)),
        );
        let wrong_unit = candidate(
            "standard:latency-unit",
            PolicyClass::Personal,
            scope(None, None),
            Applicability::default(),
            Some(numeric("p95-latency", "seconds", "5m", 1)),
        );
        let wrong_window = candidate(
            "standard:latency-window",
            PolicyClass::Personal,
            scope(None, None),
            Applicability::default(),
            Some(numeric("p95-latency", "ms", "24h", 80)),
        );

        let result = PolicyResolver::resolve(
            &request(root.path()),
            &[team, narrower, wrong_unit, wrong_window],
            &[],
        )
        .expect("constraints resolve");
        assert_eq!(
            result.personal[0].disposition,
            PersonalDisposition::CompatibleNarrowing
        );
        assert_eq!(
            result
                .personal
                .iter()
                .filter(|item| item.disposition == PersonalDisposition::Conflict)
                .count(),
            2
        );
        assert!(result
            .conflicts
            .iter()
            .any(|item| item.reason.contains("unit")));
        assert!(result
            .conflicts
            .iter()
            .any(|item| item.reason.contains("observation window")));
    }

    #[test]
    fn red_and_blue_are_conflicting_choices_not_ordered_strictness() {
        let root = TempDir::new().expect("temp project");
        let team = candidate(
            "standard:button-blue",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            Some(Constraint::Choice {
                dimension: "primary-button-color".into(),
                value: "blue".into(),
            }),
        );
        let personal = candidate(
            "standard:button-red",
            PolicyClass::Personal,
            scope(None, None),
            Applicability::default(),
            Some(Constraint::Choice {
                dimension: "primary-button-color".into(),
                value: "red".into(),
            }),
        );
        let result = PolicyResolver::resolve(&request(root.path()), &[personal, team], &[])
            .expect("choice conflict is data, not an error");
        assert_eq!(result.status, ResolutionStatus::Conflict);
        assert!(result.conflicts[0]
            .reason
            .contains("neither is inherently stricter"));
    }

    #[test]
    fn dependency_closure_and_environment_select_mandatory_gates() {
        let root = TempDir::new().expect("temp project");
        let database = candidate(
            "standard:database",
            PolicyClass::Team,
            scope(None, None),
            Applicability {
                components: BTreeSet::from(["database".into()]),
                ..Applicability::default()
            },
            None,
        );
        let production = candidate(
            "standard:production",
            PolicyClass::Team,
            scope(None, None),
            Applicability {
                environments: BTreeSet::from(["production".into()]),
                ..Applicability::default()
            },
            None,
        );
        let staging = candidate(
            "standard:staging",
            PolicyClass::Team,
            scope(None, None),
            Applicability {
                environments: BTreeSet::from(["staging".into()]),
                ..Applicability::default()
            },
            None,
        );
        let mut facts = request(root.path());
        facts.components.insert("api".into());
        facts
            .component_dependencies
            .insert("api".into(), BTreeSet::from(["database".into()]));
        facts.environment = Some("production".into());

        let result = PolicyResolver::resolve(&facts, &[staging, production, database], &[])
            .expect("declared facts resolve");
        let ids: Vec<&str> = result
            .required
            .iter()
            .map(|entry| entry.source_id.as_str())
            .collect();
        assert_eq!(ids, vec!["standard:database", "standard:production"]);
        assert!(result.trace.iter().any(|entry| {
            entry
                .facts_used
                .iter()
                .any(|fact| fact.contains("api,database"))
        }));
    }

    #[test]
    fn child_relaxation_requires_exact_independently_approved_exception() {
        let root = TempDir::new().expect("temp project");
        let parent = candidate(
            "standard:parent-limit",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            Some(numeric("bundle-size", "kb", "release", 100)),
        );
        let child = candidate(
            "standard:child-limit",
            PolicyClass::Team,
            scope(Some("web"), None),
            Applicability::default(),
            Some(numeric("bundle-size", "kb", "release", 120)),
        );
        let mut facts = request(root.path());
        facts.accepted_scope = scope(Some("web"), None);
        let without = PolicyResolver::resolve(&facts, &[parent.clone(), child.clone()], &[])
            .expect("missing exception becomes a conflict");
        assert_eq!(without.status, ResolutionStatus::Conflict);

        let exception = AuthorizedException {
            decision: reference("decision:exception"),
            parent_standard: parent.record.reference().expect("parent reference"),
            child_standard: child.record.reference().expect("child reference"),
            scope: scope(Some("web"), None),
            authority: ProvenanceAuthority::IndependentlyApproved,
            rationale: "web component has an approved temporary budget".into(),
        };
        let with = PolicyResolver::resolve(&facts, &[parent, child], &[exception])
            .expect("approved exception resolves relaxation");
        assert_eq!(with.status, ResolutionStatus::Ready);
        assert!(with.conflicts.is_empty());
    }

    #[test]
    fn business_objective_is_linked_and_left_at_judgment_boundary() {
        let root = TempDir::new().expect("temp project");
        let objective = candidate(
            "standard:retention-review",
            PolicyClass::Team,
            scope(None, None),
            Applicability {
                semantic_intents: BTreeSet::from(["improve-retention".into()]),
                ..Applicability::default()
            },
            None,
        );
        let mut facts = request(root.path());
        facts
            .relevance_hypothesis
            .irrelevant_standard_ids
            .insert(objective.record.id.clone());
        let result = PolicyResolver::resolve(&facts, &[objective], &[])
            .expect("semantic policy resolves to review");
        assert_eq!(result.status, ResolutionStatus::ReviewNeeded);
        assert_eq!(result.required.len(), 1);
        assert_eq!(
            result.required[0].boundary,
            ResolutionBoundary::JudgmentOnly
        );
        assert_eq!(result.required[0].governing_records.len(), 1);
        assert!(result.required[0]
            .rationale
            .iter()
            .any(|reason| reason.contains("governing goal")));
    }

    #[test]
    fn stale_and_unknown_required_evidence_never_resolves_ready() {
        let root = TempDir::new().expect("temp project");
        let mut stale = candidate(
            "standard:stale",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            None,
        );
        stale.evidence_state = EvidenceState::Stale;
        let stale_result = PolicyResolver::resolve(&request(root.path()), &[stale], &[])
            .expect("stale is represented");
        assert_eq!(stale_result.status, ResolutionStatus::ReviewNeeded);
        assert_eq!(stale_result.required.len(), 1);

        let mut unknown = candidate(
            "standard:unknown",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            None,
        );
        unknown.evidence_state = EvidenceState::Unknown;
        let unknown_result = PolicyResolver::resolve(&request(root.path()), &[unknown], &[])
            .expect("unknown is represented");
        assert_eq!(unknown_result.status, ResolutionStatus::Unknown);
        assert_eq!(unknown_result.required.len(), 1);
        assert!(unknown_result.required[0]
            .rationale
            .iter()
            .any(|reason| reason.contains("cannot yield a green result")));
    }

    #[test]
    fn private_experiment_cannot_weaken_or_change_team_snapshot() {
        let root = TempDir::new().expect("temp project");
        let team = candidate(
            "standard:coverage-team",
            PolicyClass::Team,
            scope(None, None),
            Applicability::default(),
            Some(Constraint::Numeric {
                metric: "coverage".into(),
                unit: "percent".into(),
                window: "change".into(),
                direction: NumericDirection::Minimum,
                threshold: 90,
            }),
        );
        let base = PolicyResolver::resolve(&request(root.path()), std::slice::from_ref(&team), &[])
            .expect("team policy resolves");
        let personal = candidate(
            "standard:coverage-experiment",
            PolicyClass::Personal,
            scope(None, None),
            Applicability::default(),
            Some(Constraint::Numeric {
                metric: "coverage".into(),
                unit: "percent".into(),
                window: "change".into(),
                direction: NumericDirection::Minimum,
                threshold: 50,
            }),
        );
        let with_personal = PolicyResolver::resolve(&request(root.path()), &[personal, team], &[])
            .expect("personal conflict remains separate");
        assert_eq!(base.required_snapshot, with_personal.required_snapshot);
        assert_eq!(base.required, with_personal.required);
        assert_eq!(with_personal.status, ResolutionStatus::Conflict);
        assert_eq!(
            with_personal.personal[0].disposition,
            PersonalDisposition::Conflict
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_parent_paths_cannot_escape_project_scope() {
        use std::os::unix::fs::symlink;

        let project = TempDir::new().expect("temp project");
        let outside = TempDir::new().expect("outside directory");
        symlink(outside.path(), project.path().join("escape")).expect("create symlink");
        let mut facts = request(project.path());
        facts.declared_paths = vec![PathBuf::from("escape/secret.rs")];
        assert!(matches!(
            PolicyResolver::resolve(&facts, &[], &[]),
            Err(PolicyError::PathEscape(_))
        ));

        facts.declared_paths = vec![PathBuf::from("../outside")];
        assert!(matches!(
            PolicyResolver::resolve(&facts, &[], &[]),
            Err(PolicyError::UnsafePath(_))
        ));
    }
}
