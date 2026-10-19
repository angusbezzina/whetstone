//! Script enforcers: the scaffold `wh init --action wire` writes under
//! `whetstone/checks/`, and the contract it states. A script is how a team
//! (or its agent) checks what no lint code or AST query can express.

use std::path::Path;

/// The directory scripts live in, relative to the repository root.
pub const DIRECTORY: &str = "whetstone/checks";
pub const README: &str = "whetstone/checks/README.md";
pub const EXAMPLE: &str = "whetstone/checks/no-conflict-markers.sh";
/// The validator command of the starter rule that runs the example.
pub const EXAMPLE_COMMAND: &str = "sh whetstone/checks/no-conflict-markers.sh";

/// The contract, as the scaffold's README states it.
pub const README_TEXT: &str = r#"# Script enforcers

When no lint code or AST query can express a rule, write a script here and
point a validator rule at it. Whetstone runs it the same way for every agent
and person: at pre-commit, at pre-push, in CI, in a sweep and in the probe
that runs before a rule is written.

## What the script is given

It runs from the repository root without a shell, with a cleared environment
(only `PATH`, `HOME`, `USER`, `TMPDIR`, `LANG`, the Rust, Node, Python, Java
and Go toolchain variables, `SSL_CERT_FILE` and `XDG_CACHE_HOME` are kept) plus:

| Variable | Meaning |
| --- | --- |
| `WH_MODE` | `staged` (pre-commit), `base` (changed since `WH_BASE`: pre-push and CI), `changed` (working-tree changes), `sweep` (a sweep of the whole repository for findings) or `all` (the whole repository: a plain `wh check` or a probe) |
| `WH_RULE` | the rule's id |
| `WH_PATHS` | a file listing the files in scope, one repository path per line, after the rule's `paths` filter; read only these |
| `WH_BASE` | the base revision in `base` mode, empty otherwise |
| `WH_EVIDENCE_DIR` | a directory kept with the run, for any extra evidence |

In `staged` mode a file on disk can differ from what is staged; read the
staged content with `git show ":$path"` when that matters.

## What the script reports

- Exit 0 passes. Any other exit fails the rule.
- A line `path:line[:col] message` is a finding at that line.
- A line `path: message` is a finding about the whole file.
- Paths are relative to the repository root and must name a file that exists.
- Anything else is kept in the evidence log only. Standard output and
  standard error are both read.
- Limits: 900 seconds (300 in a probe), 2 MiB of output (a run that exceeds
  it is unknown, never a pass) and the first 20 findings.

## Adding one

1. Write `whetstone/checks/<name>.sh` (POSIX `sh` runs anywhere; any program
   on `PATH` or inside the repository works).
2. Preview the rule: every rule `wh change` writes is probed first, and
   `--preview` shows the probe without writing anything:

   ```sh
   wh change --kind rule --record-id rule.<name> --content "<the rule>" \
     --rationale "<why>" --preview --json \
     --definition '{"type":"rule","strength":"must","enforcer":{"kind":"validator","command":"sh whetstone/checks/<name>.sh"},"paths":["src/**"]}'
   ```

3. Read the probe's `matched` count and `samples` in `data.probes`. Fix the
   script until it flags what it should and nothing else.
4. Repeat the command without `--preview` to write the draft into
   `whetstone/rulebook.json`, then stop: a person accepts it with
   `wh change --accept` or in `wh dash`.

## Safety

Commands run without a shell: no pipes, redirection, `;` or `$( )` in the
command itself (chain steps with `&&`, or put the logic in the script). The
program must be on `PATH` or a script inside the repository; a path outside
it is refused. Scripts in this directory also run at pre-commit on the staged
files, so keep them fast.

`no-conflict-markers.sh` is a working example: it reads `WH_PATHS` and
reports any line that starts a merge conflict marker.
"#;

/// The example script: leftover merge conflict markers, found by line.
pub const EXAMPLE_TEXT: &str = r#"#!/bin/sh
# A Whetstone script enforcer (contract: whetstone/checks/README.md).
# Reports every line that starts a merge conflict marker in the files in
# scope, as `path:line message`, and fails when there is any.
set -u
found=0
while IFS= read -r path; do
  [ -f "$path" ] || continue
  lines=$(grep -nIE '^(<{7}|>{7})( |$)' "$path" | cut -d: -f1) || continue
  for line in $lines; do
    echo "$path:$line leftover merge conflict marker"
    found=1
  done
done < "$WH_PATHS"
exit $found
"#;

/// The scaffold files not yet in the repository, with their contents.
pub fn missing_scaffold(root: &Path) -> Vec<(&'static str, &'static str)> {
    [(README, README_TEXT), (EXAMPLE, EXAMPLE_TEXT)]
        .into_iter()
        .filter(|(path, _)| !root.join(path).exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_flags_markers_and_nothing_in_the_scaffold_itself() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        std::fs::create_dir_all(root.join(DIRECTORY)).expect("dir");
        for (path, text) in missing_scaffold(root) {
            std::fs::write(root.join(path), text).expect("write");
        }
        std::fs::write(
            root.join("merged.txt"),
            "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> topic\n",
        )
        .expect("merged");
        let list = root.join("paths");
        std::fs::write(&list, format!("{README}\n{EXAMPLE}\nmerged.txt\n")).expect("list");
        let output = std::process::Command::new("sh")
            .arg(EXAMPLE)
            .current_dir(root)
            .env("WH_PATHS", &list)
            .output()
            .expect("run example");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(1), "{stdout}");
        assert_eq!(
            stdout,
            "merged.txt:2 leftover merge conflict marker\nmerged.txt:6 leftover merge conflict marker\n"
        );
        assert!(missing_scaffold(root).is_empty());
    }
}
