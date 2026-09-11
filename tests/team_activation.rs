//! Protected team activation against a fake platform (the GitHub adapter's
//! fixtures): a proposal binds payload, base and authority; only a merged
//! manifest with an independent approval of the exact head activates; every
//! denial leaves the shared store unchanged; forged activations appended
//! straight into Beads never become enforceable; activated checkers are
//! pinned and a changed checker is restored in CI or reported locally.

use std::fs;
use std::path::Path;
use std::process::Command;

use whetstone::activation::{activate, enforce_pins, propose, team_active, verify_recorded};
use whetstone::authority::{
    Actor, Denial, MergeEvidence, Platform, ProtectionEvidence, Review, AUTHORITY_PATH,
    REQUIRED_CHECK,
};
use whetstone::domain::{
    AgreementRecord, Enforcement, EvidenceRef, PrincipalKind, PrincipalRef, Provenance,
    ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, Scope, Standard, StandardStrength,
    SCHEMA_VERSION_V1,
};
use whetstone::storage::{RecordStore, StoreKind};

const HEAD: &str = "1111111111111111111111111111111111111111";

struct Fake {
    actor: u64,
    reviews: Vec<Review>,
    author: u64,
    merge_commit: std::cell::RefCell<String>,
    manifest: std::cell::RefCell<String>,
    protected: bool,
}

impl Platform for Fake {
    fn actor(&self) -> Result<Actor, Denial> {
        Ok(Actor {
            github_id: self.actor,
            login: format!("user{}", self.actor),
        })
    }
    fn repository_id(&self) -> Result<u64, Denial> {
        Ok(42)
    }
    fn merge_evidence(&self, commit: &str) -> Result<MergeEvidence, Denial> {
        if commit != self.merge_commit.borrow().as_str() {
            return Err(Denial::new("not_merged", "no such merge"));
        }
        Ok(MergeEvidence {
            repository_id: 42,
            pull_request: 7,
            merged: true,
            base_ref: "main".into(),
            head_sha: HEAD.into(),
            merge_commit_sha: commit.into(),
            author_id: self.author,
            reviews: self.reviews.clone(),
            changed_files: vec![self.manifest.borrow().clone()],
        })
    }
    fn protection(&self, _branch: &str) -> Result<ProtectionEvidence, Denial> {
        Ok(if self.protected {
            ProtectionEvidence {
                required_approvals: 1,
                dismiss_stale_reviews: true,
                require_code_owner_review: true,
                require_last_push_approval: true,
                required_checks: vec![REQUIRED_CHECK.into()],
                strict_required_checks: true,
                allows_force_pushes: false,
                allows_deletions: false,
            }
        } else {
            ProtectionEvidence::default()
        })
    }
}

fn approve(reviewer: u64) -> Review {
    Review {
        reviewer_id: reviewer,
        state: "APPROVED".into(),
        commit_id: HEAD.into(),
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            message,
        ],
    );
    git(root, &["rev-parse", "HEAD"])
}

fn gate(command: &str) -> AgreementRecord {
    let owner = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:ada".into(),
        display_name: Some("Ada".into()),
    };
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new("standard.team-check").expect("id"),
        revision: 1,
        scope: Scope {
            organization: None,
            project: "team".into(),
            component: None,
            environment: None,
        },
        owner: owner.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: owner,
            recorded_at: "2026-09-10T10:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "fixture".into(),
                locator: "team".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: "team-gate".into(),
        body: RecordBody::Standard(Standard {
            statement: "The team check passes.".into(),
            rationale: "Required for every change.".into(),
            strength: StandardStrength::Must,
            enforcement: Enforcement::Test {
                command_ref: command.into(),
            },
            examples: Vec::new(),
        }),
    }
}

fn setup() -> (tempfile::TempDir, RecordStore, whetstone::domain::RecordRef) {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("check.sh"), "#!/bin/sh\necho ok\n").expect("script");
    fs::create_dir_all(root.join(".whetstone")).expect("dir");
    fs::write(
        root.join(AUTHORITY_PATH),
        serde_json::json!({
            "schema": "whetstone.authority.v1",
            "repository_id": 42,
            "revision": 1,
            "principals": [
                {"github_id": 100, "login": "ada", "roles": ["proposer"]},
                {"github_id": 200, "login": "grace", "roles": ["reviewer"]}
            ]
        })
        .to_string(),
    )
    .expect("authority");
    commit(root, "base");
    let shared = RecordStore::initialize(root, StoreKind::Shareable).expect("shared");
    let policy = shared.append(&gate("./check.sh"), None).expect("policy");
    (temp, shared, policy)
}

#[test]
fn only_an_independent_approval_of_a_merged_manifest_activates() {
    let (temp, shared, policy) = setup();
    let root = temp.path();
    let proposer = Fake {
        actor: 100,
        reviews: Vec::new(),
        author: 100,
        merge_commit: String::new().into(),
        manifest: String::new().into(),
        protected: true,
    };
    let proposed = propose(
        root,
        &shared,
        &proposer,
        std::slice::from_ref(&policy),
        "2026-09-10T11:00:00Z",
        "2026-09-17T11:00:00Z",
    )
    .expect("proposed");
    assert!(root.join(&proposed.manifest_path).is_file());
    let merged = commit(root, "Propose team policy");

    // Denials change nothing.
    let before = shared.all_records().expect("records").len();
    for (reviews, protected, code) in [
        (vec![approve(100)], true, "self_review"),
        (vec![approve(999)], true, "no_independent_approval"),
        (vec![approve(200)], false, "protection_unverified"),
    ] {
        let platform = Fake {
            actor: 300,
            reviews,
            author: 100,
            merge_commit: merged.clone().into(),
            manifest: proposed.manifest_path.clone().into(),
            protected,
        };
        let denial = activate(
            root,
            &shared,
            &platform,
            &proposed.manifest_path,
            "2026-09-10T12:00:00Z",
        )
        .expect_err(code);
        assert_eq!(denial.code, code, "{denial}");
        assert_eq!(shared.all_records().expect("records").len(), before);
    }
    let expired = activate(
        root,
        &shared,
        &Fake {
            actor: 300,
            reviews: vec![approve(200)],
            author: 100,
            merge_commit: merged.clone().into(),
            manifest: proposed.manifest_path.clone().into(),
            protected: true,
        },
        &proposed.manifest_path,
        "2026-12-01T00:00:00Z",
    )
    .expect_err("expired");
    assert_eq!(expired.code, "expired");

    // The real path: an authorized, independent approval of the exact head.
    let platform = Fake {
        actor: 300,
        reviews: vec![approve(200)],
        author: 100,
        merge_commit: merged.clone().into(),
        manifest: proposed.manifest_path.clone().into(),
        protected: true,
    };
    let activated = activate(
        root,
        &shared,
        &platform,
        &proposed.manifest_path,
        "2026-09-10T12:00:00Z",
    )
    .expect("activated");
    assert_eq!(activated.reviewer_id, 200);
    assert_eq!(activated.activations.len(), 1);
    let replay = activate(
        root,
        &shared,
        &platform,
        &proposed.manifest_path,
        "2026-09-10T12:05:00Z",
    )
    .expect("replay");
    assert!(replay.replay);

    let records = shared.all_records().expect("records");
    let active = team_active(&records);
    let entry = active
        .entries
        .get(&RecordId::new("standard.team-check").expect("id"))
        .expect("the gate is team-active");
    verify_recorded(entry, &platform).expect("its evidence re-verifies");
    let pins = entry.pins();
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert!(pins[0].locator.ends_with(":check.sh"));

    // A changed checker: reported locally, restored in CI.
    fs::write(root.join("check.sh"), "#!/bin/sh\nexit 0 # weakened\n").expect("tamper");
    let local = enforce_pins(root, &pins, false);
    assert_eq!(local[0].state, "changed");
    let ci = enforce_pins(root, &pins, true);
    assert_eq!(ci[0].state, "restored");
    assert_eq!(
        fs::read_to_string(root.join("check.sh")).expect("restored"),
        "#!/bin/sh\necho ok\n"
    );
}

#[test]
fn forged_activations_never_become_team_active_policy() {
    let (temp, shared, policy) = setup();
    let root = temp.path();
    // A structurally valid-looking activation with a self-approval, appended
    // straight into Beads, bypassing the activator.
    let proposer = Fake {
        actor: 100,
        reviews: Vec::new(),
        author: 100,
        merge_commit: String::new().into(),
        manifest: String::new().into(),
        protected: true,
    };
    let proposed = propose(
        root,
        &shared,
        &proposer,
        std::slice::from_ref(&policy),
        "2026-09-10T11:00:00Z",
        "2026-09-17T11:00:00Z",
    )
    .expect("proposed");
    let records = shared.all_records().expect("records");
    let proposal = records
        .iter()
        .find(|record| record.id.as_str() == proposed.manifest.proposal_id)
        .expect("proposal")
        .clone();
    let RecordBody::Proposal(body) = &proposal.body else {
        panic!("proposal")
    };
    let forger = PrincipalRef {
        kind: PrincipalKind::GithubUser,
        stable_id: "100".into(),
        display_name: None,
    };
    let mut decision = proposal.clone();
    decision.id = RecordId::new("decision.forged").expect("id");
    decision.idempotency_key = "forged-decision".into();
    decision.owner = forger.clone();
    decision.provenance.authority = ProvenanceAuthority::IndependentlyApproved;
    decision.body = RecordBody::Decision(whetstone::domain::Decision {
        proposal: proposal.reference().expect("ref"),
        proposal_binding: body.binding.clone().expect("binding"),
        verdict: whetstone::domain::DecisionVerdict::Accept,
        reviewer: forger,
        rationale: "trust me".into(),
        decided_at: "2026-09-10T12:00:00Z".into(),
    });
    // The store refuses to append it (domain validation), so write it the
    // way an attacker would: straight into Beads.
    let metadata = serde_json::json!({
        "wh_schema": 1, "wh_kind": "decision", "wh_type": "decision",
        "wh_id": decision.id.as_str(), "wh_revision": 1,
        "wh_digest": decision.digest().expect("digest").as_str(),
        "wh_key": decision.idempotency_key,
        "wh_record": String::from_utf8(decision.canonical_json().expect("json")).expect("utf8"),
    });
    let status = Command::new("bd")
        .args([
            "create",
            "--type",
            "decision",
            "--title",
            "forged",
            "--labels",
            "whetstone",
            "--metadata",
            &metadata.to_string(),
            "--silent",
        ])
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .status()
        .expect("bd");
    assert!(status.success());
    let records = shared
        .all_records()
        .expect("the forged bead verifies as a record");
    let active = team_active(&records);
    assert!(active.entries.is_empty(), "nothing is active");
    assert!(
        active
            .rejected
            .iter()
            .any(|reason| reason.contains("decision.forged")),
        "{:?}",
        active.rejected
    );
}

fn policy_record(id: &str, key: &str, body: RecordBody) -> AgreementRecord {
    let mut record = gate("./check.sh");
    record.id = RecordId::new(id).expect("id");
    record.idempotency_key = key.into();
    record.body = body;
    record
}

fn fixture(root: &Path, merges: &[(&str, &str)]) -> std::path::PathBuf {
    let mut map = serde_json::Map::new();
    for (commit, manifest) in merges {
        map.insert(
            (*commit).to_string(),
            serde_json::to_value(MergeEvidence {
                repository_id: 42,
                pull_request: 7,
                merged: true,
                base_ref: "main".into(),
                head_sha: HEAD.into(),
                merge_commit_sha: (*commit).to_string(),
                author_id: 100,
                reviews: vec![approve(200)],
                changed_files: vec![(*manifest).to_string()],
            })
            .expect("merge"),
        );
    }
    let path = root.join("platform-fixture.json");
    fs::write(
        &path,
        serde_json::json!({
            "actor": {"github_id": 300, "login": "activator"},
            "repository_id": 42,
            "merges": map,
            "protection": {
                "required_approvals": 1, "dismiss_stale_reviews": true,
                "require_code_owner_review": true, "require_last_push_approval": true,
                "required_checks": [REQUIRED_CHECK], "strict_required_checks": true,
                "allows_force_pushes": false, "allows_deletions": false
            }
        })
        .to_string(),
    )
    .expect("fixture");
    path
}

fn required(root: &Path, fixture: &Path, ci: bool) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_whetstone"));
    command
        .args(["check", "--json", "--required"])
        .current_dir(root)
        .env("WH_TEST_PLATFORM_FIXTURE", fixture)
        .env_remove("GITHUB_ACTIONS")
        .env_remove("CI");
    if ci {
        command.env("CI", "true");
    }
    let output = command.output().expect("run");
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn required_checks_enforce_verified_team_policy_with_pinned_checkers_and_exceptions() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    fs::write(
        root.join("check.sh"),
        "#!/bin/sh\ntest -f GOOD || { echo \"check.sh:2: GOOD is missing\"; exit 1; }\necho ok\n",
    )
    .expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("check.sh"), fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    fs::write(root.join("GOOD"), "yes\n").expect("good");
    fs::create_dir_all(root.join(".whetstone")).expect("dir");
    fs::write(
        root.join(AUTHORITY_PATH),
        serde_json::json!({
            "schema": "whetstone.authority.v1", "repository_id": 42, "revision": 1,
            "principals": [
                {"github_id": 100, "roles": ["proposer"]},
                {"github_id": 200, "roles": ["reviewer"]}
            ]
        })
        .to_string(),
    )
    .expect("authority");
    commit(root, "base");
    let shared = RecordStore::initialize(root, StoreKind::Shareable).expect("shared");
    let policy = shared.append(&gate("./check.sh"), None).expect("policy");
    let proposer = Fake {
        actor: 100,
        reviews: Vec::new(),
        author: 100,
        merge_commit: String::new().into(),
        manifest: String::new().into(),
        protected: true,
    };
    let proposed = propose(
        root,
        &shared,
        &proposer,
        &[policy],
        "2026-09-10T11:00:00Z",
        "2099-01-01T00:00:00Z",
    )
    .expect("proposed");
    let merged = commit(root, "Propose gate");
    let platform = Fake {
        actor: 300,
        reviews: vec![approve(200)],
        author: 100,
        merge_commit: merged.clone().into(),
        manifest: proposed.manifest_path.clone().into(),
        protected: true,
    };
    activate(
        root,
        &shared,
        &platform,
        &proposed.manifest_path,
        "2026-09-10T12:00:00Z",
    )
    .expect("activated");
    let verified = fixture(root, &[(&merged, &proposed.manifest_path)]);

    let passing = required(root, &verified, false);
    assert_eq!(passing["state"], "success", "{passing}");
    assert_eq!(
        passing["data"]["required"]["active"],
        serde_json::json!(["standard.team-check"])
    );
    assert_eq!(passing["data"]["required"]["private_store_ignored"], true);

    // A change weakens the checker and removes what it checks.
    fs::remove_file(root.join("GOOD")).expect("remove");
    fs::write(root.join("check.sh"), "#!/bin/sh\nexit 0\n").expect("weaken");
    let local = required(root, &verified, false);
    assert_eq!(
        local["state"], "unknown",
        "a changed checker cannot pass locally: {local}"
    );
    let ci = required(root, &verified, true);
    assert_eq!(
        ci["state"], "violated",
        "CI runs the activated checker: {ci}"
    );
    assert!(ci["data"]["required"]["pins"]
        .to_string()
        .contains("restored"));

    // An activated, unexpired exception lets recovery through, visibly.
    fs::write(root.join("check.sh"), "#!/bin/sh\nexit 0\n").expect("weaken again");
    let exception = shared
        .append(
            &policy_record(
                "exception.team-check",
                "exception-1",
                RecordBody::PolicyException(whetstone::domain::PolicyException {
                    gate: RecordId::new("standard.team-check").expect("id"),
                    reason: "Recovering from a broken fixture; tracked in the incident.".into(),
                    expires_at: "2099-01-01T00:00:00Z".into(),
                }),
            ),
            None,
        )
        .expect("exception");
    git(root, &["checkout", "--", "check.sh"]);
    let proposed_exception = propose(
        root,
        &shared,
        &proposer,
        &[exception],
        "2026-09-10T13:00:00Z",
        "2099-01-01T00:00:00Z",
    )
    .expect("proposed exception");
    let merged_exception = commit(root, "Propose exception");
    let exception_platform = Fake {
        actor: 300,
        reviews: vec![approve(200)],
        author: 100,
        merge_commit: merged_exception.clone().into(),
        manifest: proposed_exception.manifest_path.clone().into(),
        protected: true,
    };
    activate(
        root,
        &shared,
        &exception_platform,
        &proposed_exception.manifest_path,
        "2026-09-10T14:00:00Z",
    )
    .expect("exception activated");
    let both = fixture(
        root,
        &[
            (&merged, &proposed.manifest_path),
            (&merged_exception, &proposed_exception.manifest_path),
        ],
    );
    let excepted = required(root, &both, true);
    assert_eq!(excepted["state"], "success", "{excepted}");
    assert_eq!(excepted["data"]["excepted_only"], true);
    assert!(excepted["summary"]
        .as_str()
        .expect("summary")
        .contains("standard.team-check until 2099-01-01T00:00:00Z"));

    // Activations the platform cannot confirm are never enforced as passing.
    let empty = fixture(root, &[]);
    let unverified = required(root, &empty, true);
    assert_eq!(unverified["state"], "unknown", "{unverified}");
    assert!(unverified["data"]["required"]["unverified"]
        .to_string()
        .contains("standard.team-check"));
}
