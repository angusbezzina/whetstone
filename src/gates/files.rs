//! The files a content check reads: the staged index for pre-commit, or the
//! working tree otherwise. Content checks (AST, design tokens, public surface)
//! read through this so staged mode never sees unstaged edits.

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

/// A file's content at a revision, or `None` when it did not exist there.
pub fn at_revision(project_root: &Path, revision: &str, path: &str) -> Option<String> {
    let spec = format!("{revision}:{path}");
    git(project_root, &["cat-file", "blob", &spec])
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// The last revision this branch pushed, for comparing public surfaces:
/// the push target, then the upstream, then `origin/HEAD`.
pub fn last_pushed_revision(project_root: &Path) -> Option<String> {
    for candidate in ["@{push}", "@{upstream}", "origin/HEAD"] {
        let spec = format!("{candidate}^{{commit}}");
        if let Ok(bytes) = git(
            project_root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &spec,
            ],
        ) {
            let commit = String::from_utf8_lossy(&bytes).trim().to_string();
            if !commit.is_empty() {
                return Some(commit);
            }
        }
    }
    None
}

/// Run `git` with a scratch object directory (the repository's objects
/// are an alternate), so computing a tree hash writes nothing into the
/// repository.
fn git_scratch(project_root: &Path, index: Option<&Path>, args: &[&str]) -> Option<String> {
    let objects = String::from_utf8(
        git(
            project_root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )
        .ok()?,
    )
    .ok()?;
    let scratch = scratch_path("objects");
    std::fs::create_dir_all(&scratch).ok()?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(project_root)
        .args(args)
        .env("GIT_OBJECT_DIRECTORY", &scratch)
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.trim());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let result = command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn scratch_path(kind: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "whetstone-{kind}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ))
}

/// The Git tree of the working tree's tracked content: the index with every
/// tracked file's current bytes (staged new files included, untracked ones
/// left out, so scratch files never change it), built in a scratch index and
/// object store so the repository is untouched.
pub fn worktree_tree(project_root: &Path) -> Option<String> {
    let index = String::from_utf8(
        git(
            project_root,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )
        .ok()?,
    )
    .ok()?;
    let index = std::path::PathBuf::from(index.trim());
    let scratch = scratch_path("index");
    if index.exists() && std::fs::copy(&index, &scratch).is_err() {
        return None;
    }
    // One scratch object store for both steps, so write-tree sees the blobs.
    let objects = String::from_utf8(
        git(
            project_root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )
        .ok()?,
    )
    .ok()?;
    let store = scratch_path("objects");
    let run = |args: &[&str]| {
        std::fs::create_dir_all(&store).ok()?;
        Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(args)
            .env("GIT_INDEX_FILE", &scratch)
            .env("GIT_OBJECT_DIRECTORY", &store)
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.trim())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let tree = run(&["add", "-u"]).and_then(|_| run(&["write-tree"]));
    let _ = std::fs::remove_file(&scratch);
    let _ = std::fs::remove_dir_all(&store);
    tree
}

/// The tree of the index (what the next commit would record).
pub fn index_tree(project_root: &Path) -> Option<String> {
    git_scratch(project_root, None, &["write-tree"])
}

/// The tree of HEAD.
pub fn head_tree(project_root: &Path) -> Option<String> {
    String::from_utf8(
        git(
            project_root,
            &["rev-parse", "--verify", "--quiet", "HEAD^{tree}"],
        )
        .ok()?,
    )
    .ok()
    .map(|tree| tree.trim().to_string())
}
