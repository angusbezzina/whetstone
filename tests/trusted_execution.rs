#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use whetstone::domain::{PrincipalKind, PrincipalRef};
use whetstone::execution::{
    configuration_digest, file_digest, manifest_digest, Argument, CheckerManifest,
    ConfigurationBinding, ExecutionApprovalEvidence, ExecutionAuthorityVerifier,
    ExecutionAuthorizationError, ExecutionGrantTarget, ExecutionRequest, ExecutionState,
    ManifestSource, TrustedExecutionService, VerifiedExecutionGrant, MANIFEST_SCHEMA,
};

#[derive(Clone)]
struct FixtureAuthority {
    grants: BTreeMap<String, VerifiedExecutionGrant>,
}

impl ExecutionAuthorityVerifier for FixtureAuthority {
    fn verify_execution_grant(
        &self,
        evidence: &ExecutionApprovalEvidence,
        _target: &ExecutionGrantTarget,
    ) -> Result<VerifiedExecutionGrant, ExecutionAuthorizationError> {
        self.grants
            .get(&evidence.locator)
            .cloned()
            .ok_or(ExecutionAuthorizationError::Rejected)
    }
}

fn principal() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "fixture-owner".into(),
        display_name: None,
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("fixture path must stay in repository")
        .to_string_lossy()
        .into_owned()
}

fn manifest(root: &Path) -> CheckerManifest {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_whetstone"));
    let rules = root.join("tests/fixtures/execution_native/rules");
    CheckerManifest {
        schema: MANIFEST_SCHEMA.into(),
        checker_id: "whetstone.ast-scanner".into(),
        checker_version: env!("CARGO_PKG_VERSION").into(),
        source: ManifestSource::Local {
            authored_by: "whetstone-k5r.9".into(),
        },
        executable: relative(root, &executable),
        executable_digest: file_digest(&executable).expect("hash built Whetstone binary"),
        args: vec![
            Argument::Literal {
                value: "scan".into(),
            },
            Argument::Input { index: 0 },
            Argument::Literal {
                value: "--project-dir".into(),
            },
            Argument::Literal { value: ".".into() },
            Argument::Literal {
                value: "--rules-dir".into(),
            },
            Argument::Configuration {
                id: "approved-rules".into(),
            },
            Argument::Literal {
                value: "--lang".into(),
            },
            Argument::Literal {
                value: "python".into(),
            },
            Argument::Literal {
                value: "--json".into(),
            },
        ],
        working_directory: ".".into(),
        configurations: vec![ConfigurationBinding {
            id: "approved-rules".into(),
            path: relative(root, &rules),
            digest: configuration_digest(&rules).expect("hash rules directory"),
        }],
        accepted_input_scopes: vec!["tests/fixtures/execution_native".into()],
        environment_allowlist: BTreeSet::new(),
        timeout_ms: 5_000,
        stdout_limit_bytes: 1_048_576,
        stderr_limit_bytes: 65_536,
        violation_exit_codes: BTreeSet::from([1]),
        citations: vec![
            "https://docs.python.org/3/tutorial/controlflow.html#defining-functions".into(),
        ],
    }
}

#[test]
fn real_whetstone_scanner_reports_known_good_and_bad_with_consumed_config() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = manifest(root);
    let digest = manifest_digest(&manifest).expect("manifest digest");
    let grant = VerifiedExecutionGrant {
        grant_reference: "fixture:execution-grant".into(),
        manifest_digest: digest,
        principal: principal(),
        authority_revision: 7,
        authority_checked_at: "2026-09-09T10:00:00Z".into(),
        expires_at: "2026-09-10T10:00:00Z".into(),
    };
    let service = TrustedExecutionService::new(FixtureAuthority {
        grants: BTreeMap::from([("grant".into(), grant)]),
    });
    let evidence = ExecutionApprovalEvidence {
        locator: "grant".into(),
    };

    for (input, expected) in [
        (
            "tests/fixtures/execution_native/good.py",
            ExecutionState::Success,
        ),
        (
            "tests/fixtures/execution_native/bad.py",
            ExecutionState::Violated,
        ),
    ] {
        let receipt = service.execute(
            root,
            &manifest,
            Some(&evidence),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &ExecutionRequest {
                inputs: vec![input.into()],
                environment: BTreeMap::new(),
            },
        );
        assert_eq!(receipt.state, expected, "{receipt:#?}");
        assert_eq!(receipt.checker_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            receipt.execution_grant_reference.as_deref(),
            Some("fixture:execution-grant")
        );
        assert_eq!(receipt.consumed_configurations.len(), 1);
        assert_eq!(receipt.consumed_configurations[0].id, "approved-rules");
    }
}
