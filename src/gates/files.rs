//! The files a content check reads: the staged index for pre-commit, or the
//! working tree otherwise. Content checks (AST) read through this so staged
//! mode never sees unstaged edits.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Where file contents come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The working tree as it is on disk.
    #[default]
    WorkingTree,
    /// The Git index: exactly what the next commit would contain.
    Staged,
}

/// Files in scope for one check, with their contents, in path order.
#[derive(Debug, Clone, Default)]
pub struct FileSet {
    pub source: Source,
    files: BTreeMap<String, Vec<u8>>,
    /// Paths deleted by the change (present at the base, gone now).
    pub deleted: Vec<String>,
}

/// Larger files are skipped by content checks, never read into memory.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

fn git(project_root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .map_err(|error| format!("git is unavailable: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn nul_list(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect()
}

impl FileSet {
    /// Drop the paths `keep` rejects, including from the deleted list.
    pub fn retain(mut self, keep: impl Fn(&str) -> bool) -> Self {
        self.files.retain(|path, _| keep(path));
        self.deleted.retain(|path| keep(path));
        self
    }

    /// Staged added, copied, modified and renamed files, read from the index.
    pub fn staged(project_root: &Path) -> Result<Self, String> {
        let names = nul_list(&git(
            project_root,
            &[
                "diff",
                "--cached",
                "--name-only",
                "--no-renames",
                "-z",
                "--diff-filter=ACMR",
            ],
        )?);
        let deleted = nul_list(&git(
            project_root,
            &[
                "diff",
                "--cached",
                "--name-only",
                "--no-renames",
                "-z",
                "--diff-filter=D",
            ],
        )?);
        let mut files = BTreeMap::new();
        for name in names.into_iter().take(MAX_FILES) {
            let spec = format!(":{name}");
            let size = String::from_utf8_lossy(&git(project_root, &["cat-file", "-s", &spec])?)
                .trim()
                .parse::<u64>()
                .unwrap_or(u64::MAX);
            if size > MAX_FILE_BYTES {
                continue;
            }
            files.insert(name, git(project_root, &["cat-file", "blob", &spec])?);
        }
        Ok(Self {
            source: Source::Staged,
            files,
            deleted,
        })
    }

    /// Named working-tree files (for example the paths a change touched).
    pub fn working(project_root: &Path, paths: &[String]) -> Self {
        let mut files = BTreeMap::new();
        let mut deleted = Vec::new();
        for path in paths.iter().take(MAX_FILES) {
            let full = project_root.join(path);
            match std::fs::symlink_metadata(&full) {
                Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES => {
                    if let Ok(bytes) = std::fs::read(&full) {
                        files.insert(path.clone(), bytes);
                    }
                }
                Ok(_) => {}
                Err(_) => deleted.push(path.clone()),
            }
        }
        Self {
            source: Source::WorkingTree,
            files,
            deleted,
        }
    }

    /// Every tracked and untracked, non-ignored file in the working tree.
    pub fn repository(project_root: &Path) -> Result<Self, String> {
        let names = nul_list(&git(
            project_root,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
        )?);
        Ok(Self::working(project_root, &names))
    }

    pub fn from_contents(source: Source, files: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            source,
            files,
            deleted: Vec::new(),
        }
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
    }

    pub fn text(&self, path: &str) -> Option<&str> {
        self.files
            .get(path)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}
