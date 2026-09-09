//! Read-only onboarding discovery and explicit installation planning.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    Ok(SetupPlan {
        project_root: root.to_string_lossy().into_owned(),
        facts,
        inferences,
        proposed_writes: vec!["private Dolt agreement store".into()],
        executable_access: vec!["none until a checker manifest is explicitly trusted".into()],
        owner_decisions: vec![
            "mission and desired outcome".into(),
            "core values and implementation philosophy".into(),
            "agreement owner and review triggers".into(),
            "one initial safeguard and its scope".into(),
        ],
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
    for entry in fs::read_dir(directory).map_err(DiscoveryError::Io)? {
        let entry = entry.map_err(DiscoveryError::Io)?;
        let path = entry.path();
        if path.is_symlink() {
            return Err(DiscoveryError::UnsafePath(path.display().to_string()));
        }
        if path.is_file() {
            files.push(path);
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
