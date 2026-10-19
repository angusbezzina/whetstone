//! Where a project is: its repository root, its identity and the private
//! state directory under the Git common directory that holds check evidence.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ProjectLayout {
    project_root: PathBuf,
    state_root: PathBuf,
    project_id: String,
}

impl ProjectLayout {
    pub fn resolve(start: &Path) -> Result<Self, LayoutError> {
        let start = start.canonicalize().map_err(LayoutError::Io)?;
        let search_root = if start.is_file() {
            start.parent().ok_or(LayoutError::ProjectRootNotFound)?
        } else {
            &start
        };
        let top = git_output(search_root, &["rev-parse", "--show-toplevel"])?;
        let project_root = PathBuf::from(top.trim())
            .canonicalize()
            .map_err(LayoutError::Io)?;
        let common = git_output(search_root, &["rev-parse", "--git-common-dir"])?;
        let common = PathBuf::from(common.trim());
        let common = if common.is_absolute() {
            common
        } else {
            search_root.join(common)
        };
        let common = common.canonicalize().map_err(LayoutError::Io)?;
        // The trailing "." is the whole-repository scope the identity has
        // always been keyed by; keeping it keeps existing evidence in place.
        let identity = format!("{}\0.", common.display());
        let project_id = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let state_root = common.join("whetstone/v1").join(&project_id[..20]);
        Ok(Self {
            project_root,
            state_root,
            project_id,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, LayoutError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(LayoutError::Io)?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| LayoutError::Git(error.to_string()))
    } else {
        Err(LayoutError::Git(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("command failed")
                .chars()
                .take(240)
                .collect(),
        ))
    }
}

#[derive(Debug)]
pub enum LayoutError {
    Io(std::io::Error),
    ProjectRootNotFound,
    /// Git could not say where the repository is, with its first line.
    Git(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::ProjectRootNotFound => write!(formatter, "no project root above the path"),
            Self::Git(message) => write!(formatter, "git: {message}"),
        }
    }
}

impl std::error::Error for LayoutError {}
