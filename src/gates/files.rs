//! The files a content check reads: the staged index for pre-commit, or the
//! working tree otherwise. Content checks (AST) read through this so staged
//! mode never sees unstaged edits. Working-tree contents are read when a
//! check asks for them, so a sweep of a large repository holds its paths,
//! not its files.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

/// Files in scope for one check, in path order.
#[derive(Debug, Clone, Default)]
pub struct FileSet {
    pub source: Source,
    /// Each path with its contents, or `None` when they are read from `root`
    /// on demand.
    files: BTreeMap<String, Option<Vec<u8>>>,
    root: PathBuf,
    /// Paths deleted by the change (present at the base, gone now).
    pub deleted: Vec<String>,
}

/// Larger files are skipped by content checks, never read into memory.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

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
        for name in names {
            let spec = format!(":{name}");
            let size = String::from_utf8_lossy(&git(project_root, &["cat-file", "-s", &spec])?)
                .trim()
                .parse::<u64>()
                .unwrap_or(u64::MAX);
            if size > MAX_FILE_BYTES {
                continue;
            }
            files.insert(name, Some(git(project_root, &["cat-file", "blob", &spec])?));
        }
        Ok(Self {
            source: Source::Staged,
            files,
            root: project_root.to_path_buf(),
            deleted,
        })
    }

    /// Named working-tree files (for example the paths a change touched).
    pub fn working(project_root: &Path, paths: &[String]) -> Self {
        let mut files = BTreeMap::new();
        let mut deleted = Vec::new();
        for path in paths {
            match std::fs::symlink_metadata(project_root.join(path)) {
                Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES => {
                    files.insert(path.clone(), None);
                }
                Ok(_) => {}
                Err(_) => deleted.push(path.clone()),
            }
        }
        Self {
            source: Source::WorkingTree,
            files,
            root: project_root.to_path_buf(),
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
            files: files
                .into_iter()
                .map(|(path, bytes)| (path, Some(bytes)))
                .collect(),
            root: PathBuf::new(),
            deleted: Vec::new(),
        }
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The contents of `path`, read from disk for a working-tree file; `None`
    /// when the path is not in the set or the file can no longer be read.
    pub fn contents(&self, path: &str) -> Option<Cow<'_, [u8]>> {
        match self.files.get(path)? {
            Some(bytes) => Some(Cow::Borrowed(bytes)),
            None => std::fs::read(self.root.join(path))
                .ok()
                .filter(|bytes| bytes.len() as u64 <= MAX_FILE_BYTES)
                .map(Cow::Owned),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}
