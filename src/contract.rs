//! The agent contract: a short, marker-delimited block in `AGENTS.md` that
//! tells any agent, of any make, how to work with this repository's rules.
//! `wh init --action wire` writes it; text outside the markers is the team's
//! and is never touched, and text inside them is Whetstone's and is rewritten.

use std::path::Path;

pub const FILE: &str = "AGENTS.md";
pub const START: &str = "<!-- whetstone:agent-contract:start -->";
pub const END: &str = "<!-- whetstone:agent-contract:end -->";

/// What wiring did to the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The file or the block was added.
    Written,
    /// The block was there and its text changed.
    Updated,
    /// The block was already exactly this.
    Unchanged,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

/// The block, with the branch a push is checked against.
pub fn block(default_branch: &str) -> String {
    format!(
        "{START}
## Whetstone rules

This repository's rules are kept by Whetstone (`wh`); every agent follows them the same way.

- Before each commit run `wh check --staged --json`; before pushing, `wh check --base {default_branch} --json`.
- Read the envelope: `state` (`success`, `violated`, `unknown`, ...), each rule in `data.gates[]` with its `state`, `failures` and `evidence`, and `permitted_actions`.
- A failed must rule blocks: repair the code, never the rule, and check again. A should rule's flag is repaired or left for a person.
- To understand a finding or a rule: `wh explain <fingerprint or rule id> --json` gives the rule, its enforcer and the decision that accepted it.
- To change a rule: `wh change --kind rule ... --probe --json` with a `--rationale`, then stop; a person accepts drafts in `wh dash`.
- To devise rules from prose (CONTRIBUTING, a style guide, review comments): `wh change --kind rule --from-file <path> --json` returns probed drafts and the candidates that still need an enforcer. Give each an enforcer, `--probe` it until it matches what it should, record the draft, and stop for a person to accept.
- When no lint code or AST query can express a rule, write a script under `whetstone/checks/` that reads the files listed in `$WH_PATHS` and prints `path:line message` findings, point a validator rule at it, probe it and record the draft. The contract is `whetstone/checks/README.md`.
- Never edit `.git/whetstone` or bypass the hooks with `--no-verify`.
{END}
"
    )
}

/// The file's new text and what changed: the block replaces the text between
/// the markers, or is appended when there are none.
pub fn merged(existing: Option<&str>, block: &str) -> (String, Outcome) {
    let Some(existing) = existing else {
        return (block.to_string(), Outcome::Written);
    };
    if let (Some(start), Some(end)) = (existing.find(START), existing.find(END)) {
        if start < end {
            let end = end + END.len();
            let tail = existing[end..]
                .strip_prefix('\n')
                .unwrap_or(&existing[end..]);
            let text = format!("{}{block}{tail}", &existing[..start]);
            let outcome = if text == existing {
                Outcome::Unchanged
            } else {
                Outcome::Updated
            };
            return (text, outcome);
        }
    }
    let separator = if existing.is_empty() {
        ""
    } else if existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    (format!("{existing}{separator}{block}"), Outcome::Written)
}

/// The branch pushes are compared with: the remote's default when Git knows
/// it, `origin/main` otherwise.
pub fn default_branch(root: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "origin/main".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_short_and_replaces_only_its_own_text() {
        let block = block("origin/main");
        assert!(block.lines().count() <= 25, "{}", block.lines().count());
        let (created, outcome) = merged(None, &block);
        assert_eq!(
            (created.as_str(), outcome),
            (block.as_str(), Outcome::Written)
        );
        let (same, outcome) = merged(Some(&created), &block);
        assert_eq!(
            (same.as_str(), outcome),
            (block.as_str(), Outcome::Unchanged)
        );

        let team = format!(
            "# Team notes\n\nKeep this.\n\n{}\nAfter.\n",
            block.replace("every agent", "EDITED")
        );
        let (fixed, outcome) = merged(Some(&team), &block);
        assert_eq!(outcome, Outcome::Updated);
        assert_eq!(
            fixed,
            format!("# Team notes\n\nKeep this.\n\n{block}\nAfter.\n")
        );

        let (appended, outcome) = merged(Some("# Ours"), &block);
        assert_eq!(outcome, Outcome::Written);
        assert_eq!(appended, format!("# Ours\n\n{block}"));
    }
}
