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
  prints `findings.json`. A host (Virgil's Messo, the reusable GitHub Action
  in this repository, an agent session) reads it and decides what to do.
  Whetstone keeps no store and no host-specific code; hosts own memory, tasks
  and the agent.
- **Any agent can follow it.** Wiring writes a short contract into
  `AGENTS.md`: how to check, read the envelope, repair, explain a finding and
  propose a rule. An agent that can run a shell command and read a file needs
  nothing else.

## Install

Install the latest release (it also links `wh`):

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
wh --help   # lists init, dash, change, check and explain
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

## Five commands

| Command | Purpose |
| --- | --- |
| `wh init` | Inspect the repository, draft the mission, principles and starter rules, and wire the Git hooks and the CI check. |
| `wh dash` | Open the local dashboard: the rulebook, the drafts, and the changelog from Git. |
| `wh change` | Draft an edit to the rulebook, retire a rule, then accept (commit) or withdraw the drafts. |
| `wh check` | Run the rules and return findings, at pre-commit, pre-push, CI or as a sweep. |
| `wh explain` | Say why a rule or a finding exists: the rule, its enforcer, what it covers, the principle it serves and the commits that accepted it. |

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

`--action agree` drafts the mission, the principles you pick and the
starter rules into `whetstone/rulebook.json`. Principles come from a
principle pack: pstack is compiled into `wh`, and a repository adds its own
packs as `whetstone/principles/<name>.json`
([schema](references/principle-pack-v1.schema.json)); name one with
`--principle <id>` or `--principle <pack>/<id>`, or write your own with
`--custom-principle`. Drafts are edits in the working tree; `wh change
--accept` commits the file, and only it. `--action setup` is `--action wire
--hooks --ci`; the hooks run the `wh` on your `PATH` and chain any hook that
was there before. Wiring also writes the agent contract into `AGENTS.md`
(between the `whetstone:agent-contract` start and end markers; text outside them is
yours) and scaffolds `whetstone/checks/` once. Every action takes
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

Every rule a change adds or re-aims (a new enforcer or new paths) is probed
before it is written: its enforcer runs once over the repository, and the
envelope's `data.probes` says whether it ran, how many files are in scope,
how many places it matched and where. A rule whose paths cover no file,
whose query does not parse, whose lint code the tool does not know, whose
script is missing or whose command cannot start is not written; pass
`--allow-unvalidated` to write it anyway. A probe runs the command, so a
`test` rule runs the test suite.

A rule's definition also takes its sweep fields: `sweep` (`discuss`, the
default, `confirm_first` or `draft_pr`: how far a host may go with a
finding), `sweep_priority` (0, most urgent, to 4; default 2) and
`sweep_unsure` (paths or a pattern, with a reason, where the rule cannot
decide). A retired rule leaves `rules` and is listed under `retired` with its
reason. Add `--preview` to any change to see its exact effect without
writing anything, the probe included. On a team, the accepted commit goes
through a pull request like any other change, and the reviewers are whoever
reviews that file.

Rules can also start from prose or from elsewhere:

```bash
wh change --from-file CONTRIBUTING.md   # probed drafts where the text names its check
wh change --from-finding <fingerprint>  # a must rule from a finding that keeps coming back
wh change --export-pack rules.pack.json # the committed rules, for another repository
wh change --import-pack rust            # a pack (a file, an https URL, rust, typescript or python) as drafts
```

`--from-text` and `--from-file` never guess with a model: a rule is drafted
only where the text names its enforcer (a formatter, a lint code, a command,
a script under `whetstone/checks/`, or a construct with a known AST query),
and the rest come back as needing one. A rule pack
(`whetstone.rule-pack.v1`) carries a content digest; an import verifies it,
probes each rule here and writes the rules as drafts. A principle pack
imported the same way is installed under `whetstone/principles/`.

## Script enforcers

When no lint code or AST query can express a rule, write a script under
`whetstone/checks/` and point a `validator` rule at it. The script runs from
the repository root without a shell and is given `WH_MODE` (`staged`,
`base`, `changed`, `sweep` or `all`), `WH_RULE`, `WH_PATHS` (a file listing
the files in scope, one per line) and `WH_BASE`. It exits 0 to pass; a line
`path:line message` is a finding at that line and `path: message` one about
the whole file. Scripts run at pre-commit on the staged files. The full
contract is `whetstone/checks/README.md`, which wiring writes with a working
example.

## Checking

```bash
wh check                    # every rule over the working tree
wh check --staged           # what pre-commit runs: AST and script rules on the staged index
wh check --base origin/main # what pre-push runs: the change since the base
wh check --ci --base "$BASE"  # the required CI check, against the committed rulebook only
wh check --rule rule.no-dbg # one rule
wh check --dry-run          # which rules and exact commands would run
wh check --sweep > findings.json
wh check --ci --base "$BASE" --format annotations  # flags as GitHub annotations
wh explain rule.no-dbg      # or a finding's fingerprint
```

A must finding fails the check; a should finding is reported and never
blocks. Unknown or unavailable evidence (a query that compiles for no
language, a missing tool, a timeout) is never a pass. Each failure names its
repair route and the command that rechecks it.

`--sweep` runs only the rules below must and prints a JSON array with one
finding per flag, one per line: `fingerprint` (sha256 of the rule, the
repository's origin and the subject, stable across runs), `subjectDigest`,
`class`, `priority`, `title`, `responsibility` (the rule) and, where
`sweep_unsure` covers the flag, `unsure` with its reason. A formatter or lint
rule whose tool can rewrite files also carries `fix`, the tool's own diff,
which applies with `git apply` (the check never applies it). The shape is in
[references](references/findings-v1.schema.json). The CI workflow uploads it
as the `scan-findings` artifact on every push to main.

`--format annotations` prints each flag as a GitHub workflow command
(`::error` for a must rule, `::warning` for a should rule); with `--base`
only lines the change touched are annotated. The CI workflow runs the
required check this way, so a reviewer sees the rule beside the code.

## The GitHub Action

`action.yml` in this repository is a reusable Action that works a sweep's
`findings.json` by class, as Messo does inside Virgil: a draft pull request
per `draft_pr` finding (applying its `fix` when it has one), an issue asking
a person to approve each `confirm_first` finding, and an issue for
discussion per `discuss` finding. A marker with the fingerprint and subject
digest makes it idempotent: an open item is updated only when the flagged
code changed, a closed one stays quiet until it does, and an `unsure`
finding is parked. It needs `wh` on the `PATH` and a token that can push
branches and open issues and pull requests. The CI workflow's
`whetstone/work` job runs it after the sweep once the repository variable
`WHETSTONE_WORK` is `on`.

## The dashboard and the decision log

`wh dash` serves a dependency-free page on localhost: Home (what needs you,
the rules in force), Rules (mission, principles, must and should rules, each
accepted, edited or new against the last commit, with Edit, Retire, Accept
and Withdraw) and Changelog (every commit that touched the rulebook, with the
retired entries) and Coverage (the parts of the repository no rule reads, by
directory and language, and the rules that read nothing). Every write is a
`wh change`. `wh dash --trail` prints the same log as a TSV trail (ts,
phase, decision, why, evidence, result); `wh dash --json` returns the
rulebook's state and revision for agents. `wh dash --export <dir>` writes the
accepted rulebook and its changelog as one static page with no script; the
CI workflow publishes it to GitHub Pages when `WHETSTONE_PAGES` is `on`.

## Contributing

Read [AGENTS.md](AGENTS.md) before changing the project. The three gates in
that file run in the pre-push hook and in CI and must pass before every push.
