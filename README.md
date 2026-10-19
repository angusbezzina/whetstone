# Whetstone

Whetstone is a repository's rulebook. It sits between an `AGENTS.md` file
(prose an agent may read, and nothing enforces) and a linter (enforced, with
no reason given and nothing an agent can act on). A Whetstone rule is
enforced and carries its rationale, and it comes with an agent protocol:
every command answers in one JSON envelope, and a sweep writes findings any
host can work.

- **The rulebook is a file.** `whetstone/rulebook.json` in the repository
  holds the mission, the chosen principles and the rules
  ([schema](references/rulebook.schema.json)). A revision is a commit, the
  decision log is Git history, and a rule change is a pull request.
- **Every rule is enforced mechanically.** A rule has one strength and one
  enforcer. `must` blocks the gate; `should` reports and never blocks. The
  enforcer is one of `ast` (a tree-sitter query), `lint` (a linter code),
  `formatter`, `test` or `validator` (a command).
- **The gates are Git's and CI's.** Pre-commit checks the staged files,
  pre-push checks what is being pushed, and CI runs the required status
  `whetstone/policy`. Every tool that commits passes through them.
- **Hosts act on findings.** `wh check --sweep` runs the rules below must and
  prints `findings.json`. A host (Virgil's Messo, a GitHub Action, an agent
  session) reads it and decides what to do. Whetstone keeps no store and no
  host-specific code; hosts own memory, tasks and the agent.

## Install

Install the latest release (it also links `wh`):

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
wh --help   # lists init, dash, change and check
```

Or build from source:

```bash
cargo build --release
mkdir -p ~/.local/bin
cp target/release/whetstone ~/.local/bin/whetstone
ln -sf ~/.local/bin/whetstone ~/.local/bin/wh
```

Whetstone needs `git` and nothing else. Releases before v0.13.0 are an
earlier product: if `wh --help` lists commands such as `extract`, `pack` or
`mcp`, an old copy is ahead on your `PATH` (check `which -a wh`).

## Four commands

| Command | Purpose |
| --- | --- |
| `wh init` | Inspect the repository, draft the mission, principles and starter rules, and wire the Git hooks and the CI check. |
| `wh dash` | Open the local dashboard: the rulebook, the drafts, and the changelog from Git. |
| `wh change` | Draft an edit to the rulebook, retire a rule, then accept (commit) or withdraw the drafts. |
| `wh check` | Run the rules and return findings, at pre-commit, pre-push, CI or as a sweep. |

Bare `wh` is read-only orientation. Every command takes `--json` and then
answers with `whetstone.command-response.v1`
([schema](references/command-response-v1.schema.json)): a `state`
(`success`, `violated`, `unknown`, `unavailable`, `needs_input`,
`needs_decision`, `stale` or `conflict`), a summary, evidence, blocking
questions and the permitted next actions. A request with `--json` never
prompts.

## A first rulebook

```bash
wh init                     # guided onboarding in the dashboard
wh init --json              # or inspect from the terminal: steps, starters, catalogue
wh init --action agree --mission "What this project exists to do." \
  --principle prove-it-works --starter all
wh change --accept --rationale "Adopt the first rulebook."
wh init --action setup      # install the pre-commit and pre-push hooks and the CI workflow
```

`--action agree` drafts the mission, the principles you pick (from the
catalogue compiled into `wh`, or `--custom-principle` in your own words) and
the starter rules into `whetstone/rulebook.json`. Drafts are edits in the
working tree; `wh change --accept` commits the file, and only it.
`--action setup` is `--action wire --hooks --ci`; the hooks run the `wh` on
your `PATH` and chain any hook that was there before. Every action takes
`--dry-run`.

## Changing a rule

```bash
wh change --kind rule --record-id rule.no-dbg \
  --content "Source has no dbg! macro." \
  --rationale "Debug output is unfinished work." \
  --definition '{"type":"rule","strength":"must","enforcer":{"kind":"ast","language":"rust","query":"((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"},"paths":["src/**"]}'
wh change --retire rule.no-dbg --rationale "Clippy's dbg_macro lint covers it."
wh change --accept --rationale "Why these edits."   # commit whetstone/rulebook.json
wh change --withdraw        # restore the file's last commit
```

A rule's definition also takes its sweep fields: `sweep` (`discuss`, the
default, `confirm_first` or `draft_pr`: how far a host may go with a
finding), `sweep_priority` (0, most urgent, to 4; default 2) and
`sweep_unsure` (paths or a pattern, with a reason, where the rule cannot
decide). A retired rule leaves `rules` and is listed under `retired` with its
reason. Add `--preview` to any change to see its exact effect without
writing anything. On a team, the accepted commit goes through a pull request like any
other change, and the reviewers are whoever reviews that file.

## Checking

```bash
wh check                    # every rule over the working tree
wh check --staged           # what pre-commit runs: AST rules on the staged index
wh check --base origin/main # what pre-push runs: the change since the base
wh check --ci --base "$BASE"  # the required CI check, against the committed rulebook only
wh check --rule rule.no-dbg # one rule
wh check --dry-run          # which rules and exact commands would run
wh check --sweep > findings.json
```

A must finding fails the check; a should finding is reported and never
blocks. Unknown or unavailable evidence (a query that compiles for no
language, a missing tool, a timeout) is never a pass. Each failure names its
repair route and the command that rechecks it.

`--sweep` runs only the rules below must and prints a JSON array with one
finding per flag, one per line: `fingerprint` (sha256 of the rule, the
repository's origin and the subject, stable across runs), `subjectDigest`,
`class`, `priority`, `title`, `responsibility` (the rule) and, where
`sweep_unsure` covers the flag, `unsure` with its reason. The CI workflow
uploads it as the `scan-findings` artifact on every push to main.

## The dashboard and the decision log

`wh dash` serves a dependency-free page on localhost: Home (what needs you,
the rules in force), Rules (mission, principles, must and should rules, each
accepted, edited or new against the last commit, with Edit, Retire, Accept
and Withdraw) and Changelog (every commit that touched the rulebook, with the
retired entries). Every write is a `wh change`. `wh dash --trail` prints the
same log as a TSV trail (ts, phase, decision, why, evidence, result);
`wh dash --json` returns the rulebook's state and revision for agents.

## Contributing

Read [AGENTS.md](AGENTS.md) before changing the project. The three gates in
that file run in the pre-push hook and in CI and must pass before every push.
