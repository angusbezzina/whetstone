//! The files a rule pack carries: what may be carried, read from the commit
//! and never the working tree, and where a pack may write. A carried file is
//! executable code entering a repository, so these rules are checked before
//! anyone is asked to agree to it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One file in a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedFile {
    pub path: String,
    /// `sha256:` of `text`.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    pub text: String,
}

fn text_digest(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

impl PackedFile {
    pub(super) fn intact(&self) -> bool {
        self.sha256 == text_digest(&self.text)
    }
}

/// Directories a pack never writes under, lower case: they run on their own,
/// or they are Whetstone's.
const RESERVED_DIRECTORIES: [&str; 3] = [".githooks/", ".github/", "whetstone/principles/"];
/// A file a pack never writes.
const RESERVED_FILE: &str = "whetstone/rulebook.json";

/// Why a pack may not hold this path, or `None` when it may. Compared
/// case-insensitively, because the filesystem may be. Paths are held to
/// plain characters because they end up in a command a person pastes.
pub fn refused_path(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') {
        return Some("a pack's paths are relative to the repository".into());
    }
    if !path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-/+@".contains(character))
    {
        return Some(
            "a pack's paths hold only letters, digits and . _ - / + @, so they are safe to paste"
                .into(),
        );
    }
    let lower = path.to_ascii_lowercase();
    if lower
        .split('/')
        .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Some("a pack's paths stay inside the repository".into());
    }
    if lower.split('/').any(|segment| segment == ".git") {
        return Some("a pack never writes into .git".into());
    }
    if let Some(directory) = RESERVED_DIRECTORIES
        .iter()
        .find(|directory| lower.starts_with(**directory))
    {
        return Some(format!("a pack never writes under {directory}"));
    }
    (lower == RESERVED_FILE).then(|| format!("a pack never writes {RESERVED_FILE}"))
}

/// The part of `path` that is a symbolic link in this repository, if any,
/// so a write never follows one out of it.
pub(super) fn through_symlink(root: &Path, path: &str) -> Option<String> {
    let mut walked = root.to_path_buf();
    let mut relative = Vec::new();
    for segment in path.split('/') {
        walked.push(segment);
        relative.push(segment);
        if std::fs::symlink_metadata(&walked)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Some(relative.join("/"));
        }
    }
    None
}

/// The repository paths `text` mentions, as exact tokens.
fn mentioned_paths(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || "._-/+@".contains(character))
    })
    .map(|token| token.strip_prefix("./").unwrap_or(token))
    .filter(|token| token.contains('/') && !token.starts_with('/'))
}

/// The files on disk that `files` mention, following what those mention in
/// turn: what a script reads, which a rulebook does not record.
pub(super) fn support_files(root: &Path, files: &[String]) -> Vec<String> {
    let mut seen = files.iter().cloned().collect::<BTreeSet<_>>();
    let mut queue = files.to_vec();
    let mut found = Vec::new();
    while let Some(path) = queue.pop() {
        let Ok(text) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };
        for token in mentioned_paths(&text) {
            if !seen.contains(token) && !token.contains("..") && root.join(token).is_file() {
                seen.insert(token.to_string());
                found.push(token.to_string());
                queue.push(token.to_string());
            }
        }
    }
    found.sort();
    found
}

/// What HEAD's tree holds, read once.
pub(super) struct Tree<'a> {
    root: &'a Path,
    /// Each tracked path and its mode.
    modes: BTreeMap<String, String>,
    read: RefCell<BTreeMap<String, Result<PackedFile, String>>>,
}

impl<'a> Tree<'a> {
    pub(super) fn at_head(root: &'a Path) -> Result<Self, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-tree", "-r", "-z", "HEAD"])
            .output()
            .map_err(|error| format!("git is unavailable: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let modes = output
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|line| {
                let line = String::from_utf8_lossy(line);
                let (meta, path) = line.split_once('\t')?;
                let mode = meta.split_whitespace().next()?;
                Some((path.to_string(), mode.to_string()))
            })
            .collect();
        Ok(Self {
            root,
            modes,
            read: RefCell::default(),
        })
    }

    pub(super) fn tracked(&self, path: &str) -> bool {
        self.modes.contains_key(path)
    }

    /// The file as committed, or why it cannot travel.
    pub(super) fn file(&self, path: &str) -> Result<PackedFile, String> {
        if let Some(known) = self.read.borrow().get(path) {
            return known.clone();
        }
        let file = self.load(path);
        self.read.borrow_mut().insert(path.into(), file.clone());
        file
    }

    fn load(&self, path: &str) -> Result<PackedFile, String> {
        let mode = self
            .modes
            .get(path)
            .ok_or_else(|| format!("{path} is not committed"))?;
        if !mode.starts_with("100") {
            return Err(format!("{path} is a link, not a file"));
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(self.root)
            .args(["cat-file", "blob", &format!("HEAD:{path}")])
            .output()
            .map_err(|error| format!("git is unavailable: {error}"))?;
        if !output.status.success() {
            return Err(format!("{path} could not be read from the commit"));
        }
        if output.stdout.contains(&0) {
            return Err(format!("{path} is a binary file"));
        }
        let text =
            String::from_utf8(output.stdout).map_err(|_| format!("{path} is a binary file"))?;
        Ok(PackedFile {
            path: path.into(),
            sha256: text_digest(&text),
            executable: mode == "100755",
            text,
        })
    }

    /// Every committed text file these files mention by its exact path,
    /// following what those mention in turn: the allow-lists and ratchets a
    /// script reads. Some are per-repository state, so a person picks.
    pub(super) fn mentioned(&self, files: &[String]) -> Vec<String> {
        let mut seen = files.iter().cloned().collect::<BTreeSet<_>>();
        let mut queue = files.to_vec();
        let mut found = Vec::new();
        while let Some(path) = queue.pop() {
            let Ok(file) = self.file(&path) else {
                continue;
            };
            for token in mentioned_paths(&file.text) {
                if self.tracked(token)
                    && !seen.contains(token)
                    && refused_path(token).is_none()
                    && self.file(token).is_ok()
                {
                    seen.insert(token.to_string());
                    found.push(token.to_string());
                    queue.push(token.to_string());
                }
            }
        }
        found.sort();
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_cannot_reach_out_of_the_repository_or_into_what_runs_on_its_own() {
        for path in [
            "/etc/passwd",
            "../outside.sh",
            "scripts/../../outside.sh",
            "scripts//x.sh",
            "./x.sh",
            ".git/hooks/pre-commit",
            ".GIT/hooks/pre-commit",
            "vendor/.git/config",
            ".githooks/pre-push",
            ".github/workflows/x.yml",
            ".GitHub/workflows/x.yml",
            "whetstone/principles/x.json",
            "whetstone/rulebook.json",
            "WHETSTONE/RULEBOOK.JSON",
            "scripts/a;b.sh",
            "scripts/a b.sh",
            "scripts/\u{200c}x.sh",
            "",
        ] {
            assert!(refused_path(path).is_some(), "{path}");
        }
        for path in [
            "scripts/check.mjs",
            "whetstone/checks/a-rule.sh",
            ".config/tool.toml",
            "packages/@scope/x.js",
        ] {
            assert_eq!(refused_path(path), None, "{path}");
        }
    }
}
