//! Read-only onboarding discovery and explicit installation planning.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::domain::ContentDigest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    Detected,
    Proposed,
    Approved,
    Installed,
    Verified,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryKind {
    Manifest,
    LintConfig,
    TestConfig,
    ContinuousIntegration,
    AgentHost,
    Automation,
    DesignSystem,
    OutcomeSource,
    ImportedMaterial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedFact {
    pub kind: DiscoveryKind,
    pub path: String,
    pub digest: ContentDigest,
    pub state: SetupState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPlan {
    pub project_root: String,
    pub facts: Vec<DetectedFact>,
    pub inferences: Vec<String>,
    pub proposed_writes: Vec<String>,
    pub executable_access: Vec<String>,
    pub owner_decisions: Vec<String>,
    pub adoption_scope: Vec<String>,
    pub known_unknowns: Vec<String>,
    pub known_debt: String,
    pub proposed_defaults: Vec<String>,
    pub platform_footprint: Vec<String>,
    pub imported_material_trust: String,
    pub remote_required: bool,
}

#[derive(Debug)]
pub enum DiscoveryError {
    InvalidRoot,
    UnsafePath(String),
    Io(std::io::Error),
}

pub fn inspect(root: &Path) -> Result<SetupPlan, DiscoveryError> {
    let root = root.canonicalize().map_err(DiscoveryError::Io)?;
    if !root.is_dir() {
        return Err(DiscoveryError::InvalidRoot);
    }
    let candidates = [
        ("Cargo.toml", DiscoveryKind::Manifest),
        ("package.json", DiscoveryKind::Manifest),
        ("pyproject.toml", DiscoveryKind::Manifest),
        ("biome.json", DiscoveryKind::LintConfig),
        ("ruff.toml", DiscoveryKind::LintConfig),
        ("clippy.toml", DiscoveryKind::LintConfig),
        ("vitest.config.ts", DiscoveryKind::TestConfig),
        ("pytest.ini", DiscoveryKind::TestConfig),
        (".github/workflows", DiscoveryKind::ContinuousIntegration),
        (".claude", DiscoveryKind::AgentHost),
        ("AGENTS.md", DiscoveryKind::AgentHost),
        ("Taskfile.yml", DiscoveryKind::Automation),
        ("Taskfile.yaml", DiscoveryKind::Automation),
        ("Makefile", DiscoveryKind::Automation),
        ("components", DiscoveryKind::DesignSystem),
        ("src/components", DiscoveryKind::DesignSystem),
        ("design-system", DiscoveryKind::DesignSystem),
        ("metrics", DiscoveryKind::OutcomeSource),
        ("analytics", DiscoveryKind::OutcomeSource),
        ("whetstone/packs", DiscoveryKind::ImportedMaterial),
        (".agents/skills", DiscoveryKind::ImportedMaterial),
        (".claude/skills", DiscoveryKind::ImportedMaterial),
    ];
    let mut facts = Vec::new();
    for (relative, kind) in candidates {
        let path = root.join(relative);
        if path.exists() {
            facts.push(fact(&root, &path, kind)?);
        }
    }
    facts.sort_by(|left, right| left.path.cmp(&right.path));
    let mut inferences = Vec::new();
    if facts.iter().any(|fact| fact.path == "Cargo.toml") {
        inferences.push("Rust may be an implementation language; confirm before adopting Rust-specific safeguards.".into());
    }
    if facts.iter().any(|fact| fact.path == "package.json") {
        inferences.push("JavaScript or TypeScript may be in scope; confirm package and generated-file boundaries.".into());
    }
    if facts.iter().any(|fact| fact.path == "pyproject.toml") {
        inferences
            .push("Python may be in scope; confirm environments and generated sources.".into());
    }
    let adoption_scope = facts
        .iter()
        .filter(|fact| fact.kind == DiscoveryKind::Manifest)
        .map(|fact| fact.path.clone())
        .collect::<Vec<_>>();
    let mut known_unknowns = Vec::new();
    for (kind, label) in [
        (DiscoveryKind::LintConfig, "native lint configuration"),
        (DiscoveryKind::TestConfig, "native test configuration"),
        (
            DiscoveryKind::ContinuousIntegration,
            "continuous integration",
        ),
        (DiscoveryKind::DesignSystem, "design-system source"),
        (DiscoveryKind::OutcomeSource, "outcome or metric source"),
    ] {
        if !facts.iter().any(|fact| fact.kind == kind) {
            known_unknowns.push(format!(
                "No {label} was detected at a bounded conventional path; confirm it rather than assuming it is absent."
            ));
        }
    }
    let imported_material_detected = facts
        .iter()
        .any(|fact| fact.kind == DiscoveryKind::ImportedMaterial);
    Ok(SetupPlan {
        project_root: root.to_string_lossy().into_owned(),
        facts,
        inferences,
        proposed_writes: vec![
            "private Dolt agreement state under the Git common directory; no working-tree files"
                .into(),
        ],
        executable_access: vec!["none until a checker manifest is explicitly trusted".into()],
        owner_decisions: vec![
            "mission".into(),
            "desired outcome".into(),
            "core values".into(),
            "implementation philosophy".into(),
            "accountable owner".into(),
            "one initial safeguard".into(),
            "initial safeguard scope".into(),
            "revision triggers".into(),
        ],
        adoption_scope,
        known_unknowns,
        known_debt: "not assessed during discovery; run an explicit scoped check after selecting and trusting the initial safeguard".into(),
        proposed_defaults: vec![
            "bounded delegation budget".into(),
            "observational checks before mutation".into(),
            "independent review for shared policy".into(),
            "bounded context retention".into(),
            "mandates remain opt-in".into(),
        ],
        platform_footprint: vec![
            "inspection writes nothing".into(),
            "agreement acceptance creates private Git-common-dir state only".into(),
        ],
        imported_material_trust: if imported_material_detected {
            "detected material remains inert until its exact content digest is separately approved"
        } else {
            "no imported starter material detected"
        }
        .into(),
        remote_required: false,
    })
}

fn fact(root: &Path, path: &Path, kind: DiscoveryKind) -> Result<DetectedFact, DiscoveryError> {
    let canonical = path.canonicalize().map_err(DiscoveryError::Io)?;
    if !canonical.starts_with(root) {
        return Err(DiscoveryError::UnsafePath(path.display().to_string()));
    }
    let digest = if canonical.is_file() {
        digest_bytes(&fs::read(&canonical).map_err(DiscoveryError::Io)?)
    } else {
        digest_directory(root, &canonical)?
    };
    Ok(DetectedFact {
        kind,
        path: canonical
            .strip_prefix(root)
            .map_err(|_| DiscoveryError::UnsafePath(path.display().to_string()))?
            .to_string_lossy()
            .into_owned(),
        digest,
        state: SetupState::Detected,
    })
}

fn digest_directory(root: &Path, directory: &Path) -> Result<ContentDigest, DiscoveryError> {
    let mut files = Vec::<PathBuf>::new();
    for entry in WalkDir::new(directory).follow_links(false).max_depth(8) {
        let entry = entry.map_err(|error| {
            DiscoveryError::Io(
                error
                    .io_error()
                    .map(|value| std::io::Error::new(value.kind(), value.to_string()))
                    .unwrap_or_else(|| std::io::Error::other(error.to_string())),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path).map_err(DiscoveryError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(DiscoveryError::UnsafePath(path.display().to_string()));
        }
        if metadata.is_file() {
            files.push(path.to_path_buf());
            if files.len() > 10_000 {
                return Err(DiscoveryError::UnsafePath(format!(
                    "inspection exceeds 10000 files under {}",
                    directory.display()
                )));
            }
        }
    }
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .map_err(|_| DiscoveryError::UnsafePath(file.display().to_string()))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(fs::read(file).map_err(DiscoveryError::Io)?);
    }
    ContentDigest::new(format!("sha256:{:x}", hasher.finalize()))
        .map_err(|_| DiscoveryError::InvalidRoot)
}

fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("sha256 is a valid digest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_is_read_only_and_distinguishes_facts_from_inferences() {
        let Ok(plan) = inspect(Path::new(env!("CARGO_MANIFEST_DIR"))) else {
            panic!("repository inspection should succeed")
        };
        assert!(plan.facts.iter().any(|fact| fact.path == "Cargo.toml"));
        assert!(plan.inferences.iter().any(|value| value.contains("may be")));
        assert!(!plan.remote_required);
        assert_eq!(plan.executable_access.len(), 1);
    }
}
