use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use whetstone::domain::{
    AgreementRecord, CoreValue, EvidenceRef, PrincipalKind, PrincipalRef, Provenance,
    ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, Scope, SCHEMA_VERSION_V1,
};
use whetstone::history::{
    AccessBoundary, HistoryInspectionRequest, HistoryInspectionService, QueryMode,
};
use whetstone::storage::{DoltRepository, ProjectLayout, StoreKind};

fn integration_enabled() -> bool {
    std::env::var("WH_DOLT_INTEGRATION").as_deref() == Ok("1")
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
    String::from_utf8(output.stdout).expect("utf-8 command output")
}

fn layout_fixture() -> (TempDir, ProjectLayout) {
    let temp = TempDir::new().expect("temp");
    let project = temp.path().join("project");
    fs::create_dir(&project).expect("project");
    command(&project, "git", &["init", "-q"]);
    let layout = ProjectLayout::resolve(&project, None).expect("layout");
    (temp, layout)
}

fn record(id: &str, request: &str, description: &str) -> AgreementRecord {
    let owner = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local-owner".into(),
        display_name: None,
    };
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id).expect("record id"),
        revision: 1,
        scope: Scope {
            organization: None,
            project: "fixture".into(),
            component: None,
            environment: None,
        },
        owner: owner.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: owner,
            recorded_at: "2026-09-09T12:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "fixture".into(),
                locator: request.into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: request.into(),
        body: RecordBody::CoreValue(CoreValue {
            name: id.into(),
            description: description.into(),
        }),
    }
}

#[test]
fn missing_history_stores_are_reported_without_initializing_project_state() {
    let (_temp, layout) = layout_fixture();
    assert!(!layout.state_root().exists());
    assert!(HistoryInspectionService::open(&layout).is_err());
    assert!(
        !layout.state_root().exists(),
        "read-only inspection must not create state"
    );
}

#[test]
fn existing_dolt_repositories_load_without_writes_and_preserve_visibility() {
    if !integration_enabled() {
        eprintln!("skipped: set WH_DOLT_INTEGRATION=1 for pinned Dolt integration");
        return;
    }
    let (_temp, layout) = layout_fixture();
    let private =
        DoltRepository::initialize(&layout.store_path(StoreKind::Private), StoreKind::Private)
            .expect("private repository");
    let shareable = DoltRepository::initialize(
        &layout.store_path(StoreKind::Shareable),
        StoreKind::Shareable,
    )
    .expect("shareable repository");
    let team = record("value.team", "team-request", "Visible to the team");
    let team_ref = private.append(&team, None).expect("append team source");
    private
        .project_selected(&shareable, std::slice::from_ref(&team_ref), &[])
        .expect("project team record");
    let private_only = record("value.private", "private-request", "HISTORY-PRIVATE-CANARY");
    private
        .append(&private_only, None)
        .expect("append private record");
    let private_root = private.root().to_path_buf();
    let shareable_root = shareable.root().to_path_buf();
    drop(private);
    drop(shareable);

    let private_status_before = command(&private_root, "dolt", &["status", "--porcelain"]);
    let shareable_status_before = command(&shareable_root, "dolt", &["status", "--porcelain"]);
    let service = HistoryInspectionService::open(&layout).expect("open existing history");

    let team_view = service
        .inspect(&HistoryInspectionRequest {
            project: "fixture".into(),
            as_of: "2026-09-10T00:00:00Z".into(),
            access: AccessBoundary::Team,
            search: None,
            history_after: None,
            page_size: 100,
            expected_snapshot: Some(service.snapshot_digest().clone()),
            redact_private_before: None,
        })
        .expect("team inspection");
    assert_eq!(team_view.decision_history.items.len(), 1);
    assert_eq!(
        team_view.decision_history.mode,
        QueryMode::Historical {
            as_of: "2026-09-10T00:00:00Z".into()
        }
    );
    assert_eq!(team_view.decision_history.items[0].reference, team_ref);

    let private_view = service
        .inspect(&HistoryInspectionRequest {
            project: "fixture".into(),
            as_of: "2026-09-10T00:00:00Z".into(),
            access: AccessBoundary::Private {
                principal: PrincipalRef {
                    kind: PrincipalKind::LocalUser,
                    stable_id: "local-owner".into(),
                    display_name: None,
                },
            },
            search: Some("HISTORY-PRIVATE-CANARY".into()),
            history_after: None,
            page_size: 100,
            expected_snapshot: None,
            redact_private_before: None,
        })
        .expect("private inspection");
    assert_eq!(private_view.decision_history.items.len(), 1);

    assert_eq!(
        command(&private_root, "dolt", &["status", "--porcelain"]),
        private_status_before
    );
    assert_eq!(
        command(&shareable_root, "dolt", &["status", "--porcelain"]),
        shareable_status_before
    );
}
