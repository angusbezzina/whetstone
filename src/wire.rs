//! `wh init --action wire`: with `--hooks`, install the Git gates; with
//! `--ci`, scaffold the required check. Either way, write the agent contract
//! into `AGENTS.md` and scaffold `whetstone/checks/` once.
//!
//! The gates are Git's: `pre-commit` runs `wh check --staged` (mechanical
//! content checks on the index) and `pre-push` runs the full `wh check` for
//! the commits being pushed. An existing hook is chained, never replaced.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::layout::ProjectLayout;
use crate::service::{InitRequest, ServiceResponse, ServiceState};

pub const CI_TEMPLATE: &str = include_str!("../assets/ci/whetstone.yml");
/// The workflow `wh init --action wire --ci` scaffolds once.
pub const CI_WORKFLOW: &str = ".github/workflows/whetstone.yml";

/// The file `wh init --action wire` writes into the tree: the CI workflow.
/// It is Whetstone's own wiring, not the project's code, so content gates
/// never read it (otherwise the commit that installs the gates is blocked
/// by it).
pub fn is_generated(path: &str) -> bool {
    path == CI_WORKFLOW
}

/// Whether a GitHub workflow runs Whetstone's check: the scaffolded one, or
/// any workflow that runs `wh check --ci` or reports `whetstone/policy`.
pub fn ci_wired(project_root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(project_root.join(".github/workflows")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) && fs::read_to_string(&path).is_ok_and(|text| {
            text.contains("wh check --ci")
                || text.contains("whetstone check --ci")
                || text.contains("whetstone/policy")
        })
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn reject_symlinked_ancestors(root: &Path, relative: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "{} is a symlink; refusing to write through it.",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn write_file(root: &Path, relative: &str, contents: &[u8]) -> Result<(), String> {
    reject_symlinked_ancestors(root, relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let staging = path.with_extension("whetstone-staging");
    fs::write(&staging, contents).map_err(|error| error.to_string())?;
    fs::rename(&staging, &path).map_err(|error| error.to_string())
}

fn error_response(request_id: String, state: ServiceState, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new_public(request_id, "init", state, summary);
    response.permitted_actions = vec!["wh init".into()];
    response
}

/// Install the Git gates and scaffold the CI workflow, as asked.
pub fn wire(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    if !request.hooks && !request.ci {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "Nothing to wire: pass --hooks for the Git gates, --ci for the required check, or both."
                .into(),
        );
    }
    let root = layout.project_root();
    let mut writes = Vec::new();
    let mut integration_notes = Vec::new();
    let mut git_hooks = Vec::new();
    if request.hooks {
        match git_hook_plan(root) {
            Ok(plan) => git_hooks = plan,
            Err(error) => return error_response(request_id, ServiceState::NeedsInput, error),
        }
        if git_hooks.is_empty() {
            integration_notes
                .push("The Git pre-commit and pre-push gates are already installed.".to_string());
        }
        for hook in &git_hooks {
            if let Some(chained) = &hook.chains {
                integration_notes.push(format!(
                    "{} already existed; it is kept as {chained} and runs first, so nothing is replaced.",
                    hook.path
                ));
            }
        }
    }
    if request.ci {
        if !root.join(CI_WORKFLOW).exists() {
            writes.push((
                CI_WORKFLOW.to_string(),
                CI_TEMPLATE.replace("__WHETSTONE_VERSION__", env!("CARGO_PKG_VERSION")),
                "scaffolded once; team-owned; protect with CODEOWNERS",
            ));
        }
        integration_notes.push(format!("Make whetstone/policy a required status check on the default branch, require a reviewing approval, and protect {CI_WORKFLOW} with CODEOWNERS. Branch protection is the team's review; Whetstone keeps no second trust root."));
    }
    // The agent contract in AGENTS.md: any agent reads it, whatever its host.
    let contract_block = crate::contract::block(&crate::contract::default_branch(root));
    let existing_contract = fs::read_to_string(root.join(crate::contract::FILE)).ok();
    let (contract_text, contract_outcome) =
        crate::contract::merged(existing_contract.as_deref(), &contract_block);
    if contract_outcome != crate::contract::Outcome::Unchanged {
        writes.push((
            crate::contract::FILE.to_string(),
            contract_text,
            "merged; text outside the Whetstone markers is kept",
        ));
    }
    // Script enforcers: the contract and one working example, once.
    let scaffold = crate::scripts::missing_scaffold(root);
    for (path, text) in &scaffold {
        writes.push((
            (*path).to_string(),
            (*text).to_string(),
            "scaffolded once; team-owned",
        ));
    }
    let agent_contract = json!({
        "path": crate::contract::FILE,
        "status": contract_outcome.label(),
    });
    let scripts = json!({
        "directory": crate::scripts::DIRECTORY,
        "contract": crate::scripts::README,
        "scaffolded": scaffold.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
    });
    let plan = writes
        .iter()
        .map(|(path, contents, ownership)| {
            json!({
                "path": path,
                "ownership": ownership,
                "bytes": contents.len(),
                "digest": digest(contents.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    if request.dry_run {
        let mut response = ServiceResponse::new_public(
            request_id,
            "init",
            ServiceState::NeedsDecision,
            format!(
                "Dry run: {} file(s) and {} hook(s) would be written; nothing was written.",
                writes.len(),
                git_hooks.len()
            ),
        );
        response.permitted_actions =
            vec!["repeat without --dry-run to write exactly these files".into()];
        response.data = json!({
            "dry_run": true,
            "writes": plan,
            "integrations": integration_notes,
            "git_hooks": git_hooks,
            "agent_contract": agent_contract,
            "scripts": scripts,
        });
        return response;
    }
    for (path, contents, _) in &writes {
        if let Err(error) = write_file(root, path, contents.as_bytes()) {
            return error_response(
                request_id,
                ServiceState::Unknown,
                format!("{path} could not be written: {error}"),
            );
        }
    }
    #[cfg(unix)]
    if scaffold
        .iter()
        .any(|(path, _)| *path == crate::scripts::EXAMPLE)
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(
            root.join(crate::scripts::EXAMPLE),
            fs::Permissions::from_mode(0o755),
        );
    }
    if let Err(error) = install_git_hooks(root, &git_hooks) {
        return error_response(
            request_id,
            ServiceState::Unknown,
            format!("The Git gates could not be installed: {error}"),
        );
    }
    let mut response = ServiceResponse::new_public(
        request_id,
        "init",
        ServiceState::Success,
        match (request.hooks, request.ci) {
            (true, true) => "The Git gates are installed and the required check is scaffolded.",
            (true, false) => "The Git pre-commit and pre-push gates are installed.",
            _ => "The required check is scaffolded.",
        }
        .to_string(),
    );
    response.permitted_actions = vec!["wh check".into()];
    response.data = json!({
        "writes": plan,
        "integrations": integration_notes,
        "git_hooks": git_hooks,
        "agent_contract": agent_contract,
        "scripts": scripts,
    });
    response
}

/// The first line of every Git hook Whetstone writes.
pub const GIT_HOOK_MARKER: &str = "# whetstone-hook v1";
/// Where an earlier hook is kept and chained from.
pub const CHAINED_SUFFIX: &str = ".pre-whetstone";

/// Whether the Git gates are installed, and where the hooks live.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct GitHooks {
    pub directory: Option<String>,
    pub pre_commit: bool,
    pub pre_push: bool,
    /// Earlier hooks Whetstone chains to.
    pub chained: Vec<String>,
}

/// The repository's hooks directory (respecting `core.hooksPath`).
pub fn git_hooks_dir(project_root: &Path) -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

pub fn git_hooks(project_root: &Path) -> GitHooks {
    let Some(directory) = git_hooks_dir(project_root) else {
        return GitHooks::default();
    };
    let ours = |name: &str| {
        fs::read_to_string(directory.join(name))
            .is_ok_and(|text| text.lines().nth(1) == Some(GIT_HOOK_MARKER))
    };
    let chained = ["pre-commit", "pre-push"]
        .into_iter()
        .filter(|name| directory.join(format!("{name}{CHAINED_SUFFIX}")).is_file())
        .map(|name| format!("{name}{CHAINED_SUFFIX}"))
        .collect();
    GitHooks {
        directory: Some(
            directory
                .strip_prefix(project_root)
                .unwrap_or(&directory)
                .display()
                .to_string(),
        ),
        pre_commit: ours("pre-commit"),
        pre_push: ours("pre-push"),
        chained,
    }
}

const WH_LOOKUP: &str = r#"if command -v wh >/dev/null 2>&1; then WH=wh
elif command -v whetstone >/dev/null 2>&1; then WH=whetstone
else
  echo "whetstone: wh is not installed, so the rules did not run. Install it, or remove this hook deliberately." >&2
  exit 1
fi"#;

/// The pre-commit hook: fast mechanical checks on the staged index.
pub fn pre_commit_hook() -> String {
    format!(
        r#"#!/bin/sh
{GIT_HOOK_MARKER}
# Installed by `wh init --action wire --hooks`. Runs the rules' fast
# mechanical checks (AST) on the staged index only, then any hook that was
# here before (kept as pre-commit{CHAINED_SUFFIX}).
hook_dir=$(dirname "$0")
if [ -x "$hook_dir/pre-commit{CHAINED_SUFFIX}" ]; then
  "$hook_dir/pre-commit{CHAINED_SUFFIX}" "$@" || exit $?
fi
{WH_LOOKUP}
exec "$WH" check --staged --hook pre-commit
"#
    )
}

/// The pre-push hook: the full check for the commits being pushed, after
/// any earlier hook.
pub fn pre_push_hook() -> String {
    format!(
        r#"#!/bin/sh
{GIT_HOOK_MARKER}
# Installed by `wh init --action wire --hooks`. Runs any hook that was here
# before (kept as pre-push{CHAINED_SUFFIX}), then the full `wh check` for the
# commits being pushed.
hook_dir=$(dirname "$0")
refs=$(cat)
if [ -x "$hook_dir/pre-push{CHAINED_SUFFIX}" ]; then
  printf '%s
' "$refs" | "$hook_dir/pre-push{CHAINED_SUFFIX}" "$@" || exit $?
fi
{WH_LOOKUP}
zero=0000000000000000000000000000000000000000
status=0
printf '%s
' "$refs" | {{
  while read -r local_ref local_sha remote_ref remote_sha; do
    [ -z "$local_sha" ] && continue
    [ "$local_sha" = "$zero" ] && continue
    if [ "$remote_sha" = "$zero" ] || [ -z "$remote_sha" ]; then
      # A new branch: compare with the remote's default branch. Git passes
      # the remote's name as $1 and its URL as $2.
      base=
      remote_head=$(git symbolic-ref -q "refs/remotes/$1/HEAD" 2>/dev/null || true)
      for candidate in "$remote_head" "$1/main" "$1/master"; do
        if [ -z "$base" ] && [ -n "$candidate" ]; then
          base=$(git merge-base "$local_sha" "$candidate" 2>/dev/null || true)
        fi
      done
    else
      base=$remote_sha
    fi
    if [ -n "$base" ]; then
      "$WH" check --base "$base" --pushed "$local_sha" --hook pre-push || exit 1
    else
      "$WH" check --pushed "$local_sha" --hook pre-push || exit 1
    fi
  done
}} || status=1
exit $status
"#
    )
}

/// One planned hook write: the file, its content, and the earlier hook it
/// moves aside to chain.
#[derive(Debug, Clone, Serialize)]
pub struct HookWrite {
    pub path: String,
    pub bytes: usize,
    pub chains: Option<String>,
    #[serde(skip)]
    pub contents: String,
}

/// The writes that install both Git gates; nothing when already installed.
pub fn git_hook_plan(project_root: &Path) -> Result<Vec<HookWrite>, String> {
    let directory = git_hooks_dir(project_root)
        .ok_or("This is not a Git repository, so there is nowhere to install the gates.")?;
    let mut plan = Vec::new();
    for (name, contents) in [
        ("pre-commit", pre_commit_hook()),
        ("pre-push", pre_push_hook()),
    ] {
        let path = directory.join(name);
        let existing = fs::read_to_string(&path).ok();
        if existing.as_deref() == Some(contents.as_str()) {
            continue;
        }
        let chains = match &existing {
            Some(text) if text.lines().nth(1) != Some(GIT_HOOK_MARKER) => {
                if directory.join(format!("{name}{CHAINED_SUFFIX}")).exists() {
                    return Err(format!(
                        "{} and {name}{CHAINED_SUFFIX} both exist; merge them by hand before installing the Whetstone gate.",
                        path.display()
                    ));
                }
                Some(format!("{name}{CHAINED_SUFFIX}"))
            }
            _ => None,
        };
        plan.push(HookWrite {
            path: path
                .strip_prefix(project_root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            bytes: contents.len(),
            chains,
            contents,
        });
    }
    Ok(plan)
}

/// Apply a hook plan: move each earlier hook aside, then write ours.
pub fn install_git_hooks(project_root: &Path, plan: &[HookWrite]) -> Result<(), String> {
    for write in plan {
        let path = project_root.join(&write.path);
        let path = path.as_path();
        if let (Some(chained), Some(directory)) = (&write.chains, path.parent()) {
            fs::rename(path, directory.join(chained)).map_err(|error| error.to_string())?;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, &write.contents).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whetstone_wiring_counts_as_generated() {
        assert!(is_generated(".github/workflows/whetstone.yml"));
        for path in ["src/lib.rs", ".github/workflows/ci.yml"] {
            assert!(!is_generated(path), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_follow_symlinked_directories() {
        let temp = tempfile::tempdir().expect("temp");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".github")).expect("symlink");
        assert!(write_file(temp.path(), ".github/workflows/whetstone.yml", b"x").is_err());
        assert!(!outside.path().join("workflows").exists());
    }

    #[test]
    fn git_hooks_chain_an_existing_hook_and_are_idempotent() {
        let temp = tempfile::tempdir().expect("temp");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .expect("git");
        let hooks = git_hooks_dir(temp.path()).expect("dir");
        fs::create_dir_all(&hooks).expect("hooks");
        fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("existing");
        let plan = git_hook_plan(temp.path()).expect("plan");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].chains.as_deref(), Some("pre-commit.pre-whetstone"));
        install_git_hooks(temp.path(), &plan).expect("install");
        let state = git_hooks(temp.path());
        assert!(state.pre_commit && state.pre_push);
        assert_eq!(state.chained, vec!["pre-commit.pre-whetstone".to_string()]);
        assert!(git_hook_plan(temp.path()).expect("again").is_empty());
    }
}
