//! Driving features: the doctor, drive proofs, flaky quarantine and mutation runs.

use super::*;

/// Evidence system marking a proof that failed then passed on one retry.
pub const QUARANTINE_EVIDENCE: &str = "whetstone_quarantine";
/// Receipt subject prefix of a mutation run: `mutation:<feature id>`.
pub const MUTATION_SUBJECT_PREFIX: &str = "mutation:";

/// The result of proving a feature with one recorded mutation applied.
#[derive(Debug, Clone, Serialize)]
pub struct MutationOutcome {
    pub feature: String,
    pub mutation: String,
    /// `killed` (the proof failed: it is real), `hollow` (the proof still
    /// passed) or `unknown`.
    pub verdict: &'static str,
    pub detail: String,
    pub proof: GateOutcome,
}

/// Apply one mutation in an isolated worktree of HEAD and drive the feature
/// there. A proof that still passes is hollow. The worktree is removed
/// afterwards; the working tree is never touched.
pub fn run_mutation(
    context: &GateContext<'_>,
    id: &RecordId,
    rule: &Rule,
    feature_id: &RecordId,
    mutation: &crate::domain::Mutation,
    index: usize,
) -> MutationOutcome {
    let unknown = |detail: String, proof: GateOutcome| MutationOutcome {
        feature: feature_id.as_str().into(),
        mutation: mutation.description.clone(),
        verdict: "unknown",
        detail,
        proof,
    };
    let base = GateOutcome::new(id, rule);
    let tree = context
        .run_dir
        .join(format!("mutation-{}-{index}", slug(feature_id.as_str())));
    let git = |args: &[&str], cwd: &Path| {
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .map_err(|error| error.to_string())
            .and_then(|output| {
                if output.status.success() {
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
                }
            })
    };
    let tree_text = tree.display().to_string();
    if let Err(error) = git(
        &["worktree", "add", "--detach", "--quiet", &tree_text, "HEAD"],
        context.project_root,
    ) {
        return unknown(
            format!("An isolated worktree could not be created: {error}"),
            base,
        );
    }
    let result = (|| {
        let target = tree.join(&mutation.path);
        let text = fs::read_to_string(&target).map_err(|error| {
            format!("{} cannot be read in the worktree: {error}", mutation.path)
        })?;
        if !text.contains(&mutation.find) {
            return Err(format!(
                "{} no longer contains the mutation's text; update the recorded mutation.",
                mutation.path
            ));
        }
        fs::write(&target, text.replacen(&mutation.find, &mutation.replace, 1))
            .map_err(|error| error.to_string())?;
        let doctor = run_doctor(&tree, context.run_dir);
        let run_id = format!("{}-mutation-{index}", context.run_id);
        let mutated = GateContext {
            project_root: &tree,
            run_dir: context.run_dir,
            run_id: &run_id,
            features: context.features,
            doctor: Some(&doctor),
            timeout: context.timeout,
            change_steps: None,
            files: context.files,
            changed: context.changed,
            surface_base: context.surface_base,
            briefs: context.briefs,
        };
        Ok(run_drive_gate(
            &mutated,
            GateOutcome::new(id, rule),
            feature_id,
        ))
    })();
    let _ = git(
        &["worktree", "remove", "--force", &tree_text],
        context.project_root,
    );
    match result {
        Err(error) => unknown(error, base),
        Ok(proof) => {
            let (verdict, detail) = match proof.state {
                VerificationAxis::Fail => (
                    "killed",
                    format!("The proof failed with the mutation applied ({}): it is real.", mutation.description),
                ),
                VerificationAxis::Pass => (
                    "hollow",
                    format!(
                        "The proof still passed with the mutation applied ({}): it is hollow and cannot satisfy a must rule until its drive steps check the behaviour.",
                        mutation.description
                    ),
                ),
                VerificationAxis::Unknown => ("unknown", format!("The mutated drive was inconclusive: {}", proof.summary)),
            };
            MutationOutcome {
                feature: feature_id.as_str().into(),
                mutation: mutation.description.clone(),
                verdict,
                detail,
                proof,
            }
        }
    }
}

/// Run the project driver's read-only health check.
pub fn run_doctor(project_root: &Path, run_dir: &Path) -> DoctorResult {
    let Some(driver) = driver_path(project_root) else {
        return DoctorResult {
            ok: false,
            detail: format!("No verification driver exists at {DRIVER_RELATIVE}."),
        };
    };
    let node = match resolve_program(project_root, "node") {
        Ok(node) => node,
        Err(error) => {
            return DoctorResult {
                ok: false,
                detail: error,
            }
        }
    };
    let environment = gate_environment(&[("WH_EVIDENCE_DIR", run_dir.display().to_string())]);
    match crate::execution::run_bounded(
        &node,
        &[driver, "doctor".into(), "--json".into()],
        project_root,
        &environment,
        Duration::from_secs(60),
        OUTPUT_LIMIT,
        OUTPUT_LIMIT,
    ) {
        Ok(result) => {
            let parsed = result
                .stdout
                .text
                .lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Value>(line).ok());
            let ok = result.status.success()
                && parsed
                    .as_ref()
                    .and_then(|value| value.get("ok"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            DoctorResult {
                ok,
                detail: parsed
                    .as_ref()
                    .and_then(|value| value.get("detail"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| last_meaningful_line(&result.stderr.text)),
            }
        }
        Err(error) => DoctorResult {
            ok: false,
            detail: format!("The driver could not start: {error}"),
        },
    }
}

pub(super) fn run_drive_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    feature_id: &RecordId,
) -> GateOutcome {
    let Some(feature) = context.features.get(feature_id) else {
        let outcome = outcome.unknown(format!(
            "Feature {} is not in force; accept it before it can be proven.",
            feature_id.as_str()
        ));
        return finish(context, outcome, None, json!({}));
    };
    if feature.drive_steps.is_empty() {
        let outcome = outcome.unknown(format!(
            "Feature {} has no drive steps to prove it.",
            feature.name
        ));
        return finish(context, outcome, None, json!({}));
    }
    let Some(driver) = driver_path(context.project_root) else {
        let outcome = outcome.unknown(format!(
            "No verification driver exists at {DRIVER_RELATIVE}; run wh init --action wire."
        ));
        return finish(context, outcome, None, json!({}));
    };
    match context.doctor {
        Some(doctor) if doctor.ok => {}
        Some(doctor) => {
            let outcome = outcome.unknown(format!(
                "Doctor failed before driving: {}. A drive against an unhealthy instance is not evidence.",
                doctor.detail
            ));
            return finish(context, outcome, None, json!({"doctor": doctor}));
        }
        None => {
            let outcome = outcome.unknown("Doctor did not run before driving.");
            return finish(context, outcome, None, json!({}));
        }
    }
    let slug = slug(&outcome.id);
    let steps_file = format!("{slug}.steps.json");
    let change_steps = context
        .change_steps
        .filter(|(id, _)| *id == feature_id)
        .map_or(&[][..], |(_, steps)| steps);
    let all_steps = feature
        .drive_steps
        .iter()
        .chain(change_steps)
        .cloned()
        .collect::<Vec<_>>();
    let steps = json!({"feature": feature_id.as_str(), "name": feature.name, "steps": all_steps, "accepted_steps": feature.drive_steps.len(), "change_steps": change_steps, "proof": feature.proof});
    if let Err(error) = serde_json::to_vec_pretty(&steps)
        .map_err(|error| error.to_string())
        .and_then(|bytes| write_evidence(context, &steps_file, &bytes))
    {
        return outcome.unknown(format!("Evidence could not be written: {error}"));
    }
    let shots = context.run_dir.join(format!("{slug}.drive"));
    if let Err(error) = fs::create_dir_all(&shots) {
        return outcome.unknown(format!("Evidence could not be written: {error}"));
    }
    let node = match resolve_program(context.project_root, "node") {
        Ok(node) => node,
        Err(error) => return finish(context, outcome.unknown(error), None, json!({})),
    };
    let environment = gate_environment(&[
        ("WH_EVIDENCE_DIR", shots.display().to_string()),
        ("WH_RUN_ID", context.run_id.to_string()),
    ]);
    let result = crate::execution::run_bounded(
        &node,
        &[
            driver,
            "prove".into(),
            "--steps-file".into(),
            context.run_dir.join(&steps_file).display().to_string(),
            "--json".into(),
        ],
        context.project_root,
        &environment,
        context.timeout,
        OUTPUT_LIMIT,
        OUTPUT_LIMIT,
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let outcome = outcome.unknown(format!("The driver could not start: {error}"));
            return finish(context, outcome, None, json!({}));
        }
    };
    outcome.exit_code = result.status.code();
    outcome.elapsed_ms = u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX);
    outcome.program = Some(node.display().to_string());
    outcome.program_digest = program_digest(&context.project_root.join(DRIVER_RELATIVE));
    let log = format!("{}{}", result.stdout.text, result.stderr.text);
    let report = result
        .stdout
        .text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok());
    // Evidence the driver actually produced, bound by digest.
    let mut produced = Vec::new();
    if let Ok(entries) = fs::read_dir(&shots) {
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if let Ok(bytes) = fs::read(&path) {
                if bytes.is_empty() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                produced.push(EvidenceRef {
                    system: EVIDENCE_SYSTEM.into(),
                    locator: format!("{}/{slug}.drive/{name}", context.run_id),
                    digest: ContentDigest::new(sha256_hex(&bytes)).ok(),
                });
            }
        }
    }
    outcome.evidence.extend(produced.clone());
    let reported = report
        .as_ref()
        .and_then(|report| report.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    outcome.failures = report
        .as_ref()
        .and_then(|report| report.get("failures"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_FAILURES)
                .map(|item| ArtifactFailure {
                    location: item
                        .get("location")
                        .and_then(Value::as_str)
                        .unwrap_or("drive step")
                        .to_string(),
                    message: item
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("step failed")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    if result.timed_out {
        outcome = outcome.unknown("The drive timed out; a timeout is not a result.");
    } else if reported == "unreachable" {
        let prerequisite = report
            .as_ref()
            .and_then(|report| report.get("prerequisite"))
            .and_then(Value::as_str)
            .unwrap_or("unstated");
        let detail = report
            .as_ref()
            .and_then(|report| report.get("detail"))
            .and_then(Value::as_str)
            .unwrap_or("a prerequisite is not met");
        outcome = outcome.unknown(format!("{UNREACHABLE_PREFIX}{prerequisite} ({detail})"));
        outcome.failures = vec![ArtifactFailure {
            location: format!("prerequisite {prerequisite}"),
            message: detail.to_string(),
        }];
    } else if reported == "pass" && result.status.success() {
        if produced.is_empty() {
            outcome = outcome.unknown(
                "The driver reported success but produced no evidence; that is not a pass.",
            );
        } else {
            outcome.state = VerificationAxis::Pass;
            outcome.summary = if change_steps.is_empty() {
                format!(
                    "Drove {} step(s); proof: {}",
                    feature.drive_steps.len(),
                    feature.proof
                )
            } else {
                format!(
                    "Drove {} accepted and {} change-specific step(s) ({}); proof: {}",
                    feature.drive_steps.len(),
                    change_steps.len(),
                    change_steps.join("; "),
                    feature.proof
                )
            };
        }
    } else if reported == "fail" {
        outcome.state = VerificationAxis::Fail;
        if outcome.failures.is_empty() {
            outcome.failures.push(ArtifactFailure {
                location: "drive".into(),
                message: last_meaningful_line(&log),
            });
        }
        outcome.summary = format!("Driving {} did not reach the proof.", feature.name);
    } else {
        outcome = outcome.unknown(format!(
            "The driver returned no verdict: {}",
            last_meaningful_line(&log)
        ));
    }
    finish(
        context,
        outcome,
        Some(&log),
        json!({"feature": feature_id.as_str(), "report": report}),
    )
}
