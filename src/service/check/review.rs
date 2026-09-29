//! What a check binds to beyond rules: the pushed commit, review attestations, briefs and raised hands.

use super::*;

/// Why the working tree is not exactly the commit being pushed, if it is
/// not. The check runs commands and reads files in the working tree, so it
/// can only vouch for a push when that tree is the pushed commit.
pub(super) fn pushed_mismatch(project: &Path, pushed: &str) -> Option<String> {
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

/// The latest attestation for exactly this rule revision that covers the
/// content being checked: its reviewed tree is the checked tree (the index
/// at pre-commit, the pushed commit at pre-push, the working tree
/// otherwise). An attestation without a tree holds at its commit only.
pub(super) fn current_attestation<'a>(
    agreement: Option<&'a AgreementState>,
    selected: &SelectedRule,
    head: Option<&str>,
    checked_tree: Option<&str>,
) -> Option<&'a Attestation> {
    agreement?
        .records()
        .iter()
        .filter_map(|record| match &record.body {
            RecordBody::Attestation(body) if body.rule == selected.reference => {
                let covers = match (&body.tree, checked_tree) {
                    (Some(tree), Some(checked)) => tree == checked,
                    (Some(_), None) => false,
                    (None, _) => head.is_some_and(|head| head == body.commit),
                };
                covers.then_some(body)
            }
            _ => None,
        })
        .max_by(|left, right| left.attested_at.cmp(&right.attested_at))
}

pub(super) fn attestation_evidence(
    selected: &SelectedRule,
    attestation: &Attestation,
    snapshot: &crate::verification::SnapshotBinding,
) -> VerificationEvidence {
    VerificationEvidence::Attestation(crate::verification::VerifiedAttestation {
        requirement_id: format!("gate:{}", selected.id.as_str()),
        snapshot: snapshot.clone(),
        state: match attestation.verdict {
            AttestationVerdict::Pass => AttestationState::Success,
            AttestationVerdict::Fail => AttestationState::Violated,
        },
        evidence: EvidencePointer {
            source: "whetstone_attestation".into(),
            locator: format!(
                "{}@{}",
                attestation.reviewer.label(),
                attestation.attested_at
            ),
            digest: digest_bytes(attestation.notes.as_bytes()),
        },
        observed_at_unix: unix_now(),
        summary: format!("{}: {}", attestation.reviewer.label(), attestation.notes),
        findings: Vec::new(),
    })
}

pub(super) fn brief_views(state: &AgreementState) -> Vec<BriefView> {
    state
        .records()
        .iter()
        .filter_map(|record| match &record.body {
            RecordBody::Brief(body) => Some(BriefView {
                area: body.area.clone(),
                recorded_at: body.recorded_at.clone(),
                commit: body.commit.clone(),
                skills: body.skills.clone(),
            }),
            _ => None,
        })
        .collect()
}

pub(super) fn receipt_response(
    request_id: String,
    summary: String,
    kind: &str,
    reference: RecordRef,
    extra: Value,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(request_id, "check", ServiceState::Success, summary);
    response.permitted_actions = vec!["wh check".into(), "wh dash".into()];
    response.data = json!({ kind: reference, "detail": extra });
    response
}

pub(super) fn record_attestation(
    layout: &ProjectLayout,
    request_id: String,
    attest: &AttestRequest,
) -> ServiceResponse {
    let loaded = match load_records(layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("check", request_id, error),
    };
    let state = AgreementState::from_records(loaded.union().0);
    let Ok(rule_id) = RecordId::new(attest.rule.as_str()) else {
        return unknown_response(
            "check",
            request_id,
            format!("{} is not a rule id.", attest.rule),
        );
    };
    let Some(rule_record) = state.in_force(&rule_id) else {
        return unknown_response(
            "check",
            request_id,
            format!(
                "{} is not a rule in force; only an accepted rule can be attested.",
                attest.rule
            ),
        );
    };
    let Some(head) = crate::gates::head_commit(layout.project_root()) else {
        return unknown_response(
            "check",
            request_id,
            "An attestation binds to a commit, and this repository has none yet.".into(),
        );
    };
    let notes = match bounded_input(Some(attest.notes.clone()), "review notes", 4_000) {
        Ok(notes) => notes,
        Err(question) => return unknown_response("check", request_id, question),
    };
    let reference = match rule_record.reference() {
        Ok(reference) => reference,
        Err(error) => return domain_response("check", request_id, error),
    };
    let attested_at = utc_now();
    let tree = crate::gates::files::worktree_tree(layout.project_root());
    // A new verdict, note or reviewed tree is a new attestation; an exact
    // retry returns the one already recorded.
    let key = format!(
        "attest:{request_id}:{}:{head}:{}",
        rule_id.as_str(),
        key_suffix(
            &format!(
                "{:?}\0{}\0{}\0{}",
                attest.verdict,
                attest.reviewer.label(),
                notes,
                tree.as_deref().unwrap_or_default()
            ),
            16
        )
    );
    let evidence = attest
        .evidence
        .as_ref()
        .map(|locator| {
            let local = layout.project_root().join(locator);
            EvidenceRef {
                system: "review_evidence".into(),
                locator: locator.chars().take(500).collect(),
                digest: (!locator.contains("://"))
                    .then(|| fs::read(&local).ok())
                    .flatten()
                    .map(|bytes| digest_bytes(&bytes)),
            }
        })
        .into_iter()
        .collect();
    let record = match receipt_record(
        layout,
        format!("verification.attest_{}", key_suffix(&key, 24)),
        key,
        "whetstone:review",
        ProvenanceKind::ExternalObservation,
        &attested_at,
        EvidenceRef {
            system: "review".into(),
            locator: attest.reviewer.label(),
            digest: None,
        },
        RecordBody::Attestation(Attestation {
            rule: reference,
            commit: head.clone(),
            tree,
            reviewer: attest.reviewer.clone(),
            verdict: attest.verdict,
            notes,
            evidence,
            attested_at: attested_at.clone(),
        }),
    ) {
        Ok(record) => record,
        Err(error) => return domain_response("check", request_id, error),
    };
    let already = RecordStore::open_existing(&layout.private_store(), StoreKind::Private)
        .ok()
        .and_then(|store| {
            store
                .by_idempotency_key(&record.idempotency_key)
                .ok()
                .flatten()
        })
        .is_some();
    match append_private(layout, &record) {
        Ok(reference) => receipt_response(
            request_id,
            format!(
                "The review of {} by {} {} for commit {}.",
                attest.rule,
                attest.reviewer.label(),
                if already {
                    "was already recorded (the same verdict, notes and content)"
                } else {
                    "was recorded"
                },
                &head[..12]
            ),
            "attestation",
            reference,
            json!({"verdict": attest.verdict, "commit": head}),
        ),
        Err(error) => storage_error("check", request_id, error),
    }
}

pub(super) fn record_brief(
    layout: &ProjectLayout,
    request_id: String,
    brief: &BriefRequest,
) -> ServiceResponse {
    let (area, notes, skills, reuse, risks) = match (
        bounded_input(Some(brief.area.clone()), "brief area", 500),
        bounded_input(Some(brief.notes.clone()), "brief notes", 8_000),
        bounded_list(&brief.skills, "skills", 8, 100),
        bounded_list(&brief.reuse, "reuse", 32, 1_000),
        bounded_list(&brief.risks, "risks", 32, 1_000),
    ) {
        (Ok(area), Ok(notes), Ok(skills), Ok(reuse), Ok(risks)) => {
            (area, notes, skills, reuse, risks)
        }
        (area, notes, skills, reuse, risks) => {
            let question = area
                .err()
                .or_else(|| notes.err())
                .or_else(|| skills.err())
                .or_else(|| reuse.err())
                .or_else(|| risks.err())
                .unwrap_or_default();
            return unknown_response("check", request_id, question);
        }
    };
    if skills.is_empty() {
        return unknown_response(
            "check",
            request_id,
            "Name the pstack skills the brief came from (--skill how, --skill why, --skill blast-radius).".into(),
        );
    }
    let recorded_at = utc_now();
    let key = format!("brief:{request_id}:{area}:{recorded_at}");
    let record = match receipt_record(
        layout,
        format!("verification.brief_{}", key_suffix(&key, 24)),
        key,
        "whetstone:brief",
        ProvenanceKind::ImportedNote,
        &recorded_at,
        EvidenceRef {
            system: "pstack".into(),
            locator: skills.join(","),
            digest: None,
        },
        RecordBody::Brief(Brief {
            area: area.clone(),
            commit: crate::gates::head_commit(layout.project_root()),
            skills,
            reuse,
            risks,
            notes,
            recorded_at: recorded_at.clone(),
        }),
    ) {
        Ok(record) => record,
        Err(error) => return domain_response("check", request_id, error),
    };
    match append_private(layout, &record) {
        Ok(reference) => receipt_response(
            request_id,
            format!("The brief for {area} was recorded; it appears in the trail."),
            "brief",
            reference,
            json!({"area": area}),
        ),
        Err(error) => storage_error("check", request_id, error),
    }
}

pub(crate) fn raise_hand(
    layout: &ProjectLayout,
    request_id: String,
    hand: &HandRequest,
) -> ServiceResponse {
    let (question, tried, recommendation) = match (
        bounded_input(Some(hand.question.clone()), "question", 2_000),
        bounded_input(Some(hand.tried.clone()), "what was tried", 4_000),
        bounded_input(Some(hand.recommendation.clone()), "recommendation", 4_000),
    ) {
        (Ok(question), Ok(tried), Ok(recommendation)) => (question, tried, recommendation),
        (question, tried, recommendation) => {
            return unknown_response(
                "check",
                request_id,
                question
                    .err()
                    .or_else(|| tried.err())
                    .or_else(|| recommendation.err())
                    .unwrap_or_default(),
            )
        }
    };
    let rule = match hand.rule.as_deref().map(RecordId::new).transpose() {
        Ok(rule) => rule,
        Err(error) => return domain_response("check", request_id, error),
    };
    let key = format!(
        "hand:{request_id}:{}",
        digest_bytes(question.as_bytes()).as_str()
    );
    // An exact retry returns the hand already raised; one question, one issue.
    if let Ok(store) = RecordStore::open_existing(&layout.private_store(), StoreKind::Private) {
        if let Ok(Some(existing)) = store.by_idempotency_key(&key) {
            if let RecordBody::HandRaise(body) = &existing.body {
                let mut response = ServiceResponse::new(
                    request_id,
                    "check",
                    ServiceState::NeedsDecision,
                    format!(
                        "This hand was already raised as {}; take other ready work (bd ready).",
                        body.issue
                    ),
                );
                response.permitted_actions = vec!["bd ready".into()];
                response.data = json!({"issue": body.issue, "idempotent_replay": true});
                return response;
            }
        }
    }
    let description = crate::hands::issue_description(
        &question,
        &tried,
        &recommendation,
        hand.trigger.label(),
        rule.as_ref().map(RecordId::as_str),
    );
    let issue = match crate::hands::file_issue(
        layout.project_root(),
        &format!("Owner decision needed: {question}"),
        &description,
    ) {
        Ok(issue) => issue,
        Err(error) => {
            let mut response = ServiceResponse::new(
                request_id,
                "check",
                ServiceState::Unavailable,
                format!("The hand could not be raised in Beads: {error}"),
            );
            response.permitted_actions = vec!["bd init, then raise the hand again".into()];
            return response;
        }
    };
    let raised_at = utc_now();
    let record = match receipt_record(
        layout,
        format!("verification.hand_{}", key_suffix(&key, 24)),
        key,
        "whetstone:agent",
        ProvenanceKind::HumanAuthored,
        &raised_at,
        EvidenceRef {
            system: "beads".into(),
            locator: issue.clone(),
            digest: None,
        },
        RecordBody::HandRaise(HandRaise {
            issue: issue.clone(),
            trigger: hand.trigger,
            rule,
            question: question.clone(),
            tried,
            recommendation,
            raised_at: raised_at.clone(),
        }),
    ) {
        Ok(record) => record,
        Err(error) => return domain_response("check", request_id, error),
    };
    let reference = match append_private(layout, &record) {
        Ok(reference) => reference,
        Err(error) => return storage_error("check", request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "check",
        ServiceState::NeedsDecision,
        format!("A hand was raised for the owner as {issue} (labelled human). Take other ready work now; do not guess the answer."),
    );
    response.permitted_actions = vec![
        "bd ready".into(),
        format!("bd human list (the owner answers {issue})"),
    ];
    response.data = json!({"issue": issue, "hand": reference, "question": question});
    response
}
