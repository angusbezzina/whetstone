# Script enforcers

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
| `WH_PATHS` | a file listing the files in scope, one repository path per line, after the rule's `paths` and `exclude` filters; read only these |
| `WH_BASE` | the base revision in `base` mode, empty otherwise |
| `WH_EVIDENCE_DIR` | a directory kept with the run, for any extra evidence |

In `staged` mode a file on disk can differ from what is staged; read the
staged content with `git show ":$path"` when that matters.

A script that judges only a change needs no `WH_MODE` guard: give its rule
`"mode": "change"`, and only a check against a base runs it (a sweep runs
it in `sweep` mode to count its flags, and files no finding).

## What the script reports

- Exit 0 passes. Exit 69 means the script could not reach a verdict (a tool,
  permission or service it needs is unavailable): the rule is unknown, which
  is never a pass, and no finding is read. Any other exit fails the rule.
- A line `path:line[:col] message` is a finding at that line.
- A line `path: message` is a finding about the whole file.
- Paths are relative to the repository root and must name a file that exists.
- Anything else is kept in the evidence log only. Standard output and
  standard error are both read.
- Limits: 900 seconds (300 in a probe) and 2 MiB of output (a run that
  exceeds it is unknown, never a pass). A sweep keeps every finding; any
  other check lists the first 20 of each rule and says how many it left out.

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
