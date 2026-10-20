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
  prints `findings.json`. A host reads it and decides what to do. A host
  can be an agent session, the reusable GitHub Action in this repository,
  or a maintenance controller such as Messo in the Virgil project.
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
Building from source overwrites `~/.local/bin/whetstone`, including a copy
the installer put there.

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
prompts. When standard output is not a terminal (a pipe, a file, an agent's
shell), commands print the envelope even without `--json`. The exit code
follows the state: 0 success, 1 violated, 2 a bad argument, 3 unknown,
4 unavailable, 5 needs a decision (a preview), 6 needs input, 7 stale,
8 conflict.

**For agents.** An agent needs a shell and nothing else. It runs commands
with `--json` and reads the envelope. It follows the contract that wiring
writes into the repository's `AGENTS.md`. It asks `wh explain <rule id or
fingerprint>` why a rule or a finding exists. It proposes a rule by drafting
it with `wh change --kind rule …`, and leaves accepting to a person. A draft
takes force only when a person runs `wh change --accept` or accepts it in
the dashboard; on a team, that commit goes through a pull request.

## A first rulebook

```bash
wh init                     # guided onboarding in the dashboard
wh init --no-open           # or the same steps and exact commands in the terminal
wh init --json              # the steps, starters and catalogue as JSON
wh init --action agree --mission "What this project exists to do." \
  --principle starting --starter all
wh change --accept --rationale "Adopt the first rulebook."
wh init --action setup      # install the pre-commit and pre-push hooks and the CI workflow
```

`--action agree` drafts the mission, the principles you pick and the
starter rules into `whetstone/rulebook.json`. Principles come from a
principle pack. Two are compiled into `wh`: pstack, Lauren Tan's principles,
and `mattpocock`, the lessons of Matt Pocock's skills in Whetstone's words.
A repository adds its own packs as `whetstone/principles/<name>.json`
([schema](references/principle-pack-v1.schema.json)). Name one with
`--principle <id>` or `--principle <pack>/<id>`, take the ten the packs
offer as a starting point with `--principle starting` (the dashboard ticks
them for you), or write your own with `--custom-principle`. Every picked
principle is a record you can edit or retire like a rule.

Drafts are edits in the working tree; `wh change --accept` commits the
file, and only it. `--action setup` is `--action wire --hooks --ci`; the
hooks run the `wh` on your `PATH` and chain any hook that was there before.
Wiring also writes the agent contract into `AGENTS.md` (between the
`whetstone:agent-contract` start and end markers; text outside them is
yours) and scaffolds `whetstone/checks/` once. Commit what wiring wrote
(`.github/workflows/whetstone.yml`, `AGENTS.md`, `whetstone/checks/`) like
any other change. Every action takes `--dry-run`.

## Changing a rule

Each line below is a separate action, not a sequence to paste:

```bash
wh change --kind rule --record-id rule.no-dbg \
  --content "Source has no dbg! macro." \
  --rationale "Debug output is unfinished work." \
  --definition '{"type":"rule","strength":"must","enforcer":{"kind":"ast","language":"rust","query":"((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"},"paths":["src/**"]}'
wh change --retire rule.no-dbg --rationale "Clippy's dbg_macro lint covers it."
wh change --accept --rationale "Why these edits."   # commit whetstone/rulebook.json
wh change --withdraw        # restore the file's last commit
```

Add `--serves <principle id>` to a rule change to name the principle the
rule serves (one the rulebook holds). It becomes the rule's source;
`wh explain` prints it, the dashboard shows it under Serves, and every
sweep finding carries it, so a host can quote the why with the flag.

No tool checks a principle, so Whetstone enforces the chain instead: every
principle should have a rule that serves it, and every rule should serve a
principle. `wh check` and the dashboard name the principles no rule serves
and the rules that serve none. `wh explain <principle id>` lists the rules
that serve a principle. A principle that rules still serve cannot be
retired until they serve another one or are retired themselves. A team that
wants the chain whole before anything is committed sets strict mode with
`wh change --require-served true` (the rulebook's `require_served` field).
`wh change --accept` then refuses a rulebook with a gap and lists it.

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
decide). Only a person widens a rule's sweep class: a `wh change` from an
agent (no terminal, or an agent host's environment) may keep or narrow it,
and a person sets it in a terminal or the dashboard. A retired rule leaves `rules` and is listed under `retired` with its
reason. Add `--preview` to any change to see its exact effect without
writing anything, the probe included. On a team, the accepted commit goes
through a pull request like any other change, and the reviewers are whoever
reviews that file.

Rules can also start from prose or from elsewhere:

```bash
wh change --from-file CONTRIBUTING.md   # probed drafts where the text names its check
wh change --from-finding <fingerprint>  # a must rule from a finding that keeps coming back
wh change --export-pack rules.pack.json # the committed rules and principles, for another repository
wh change --export-pack team.json --principles  # your own principles as a principle pack, like pstack
wh change --import-pack rust            # a pack (a file, an https URL, rust, typescript or python) as drafts
```

`--from-text` and `--from-file` never guess with a model: a rule is drafted
only where the text names its enforcer (a formatter, a lint code, a command,
a script under `whetstone/checks/`, or a construct with a known AST query),
and the rest come back as needing one. A rule pack
(`whetstone.rule-pack.v1`) carries a content digest; an import verifies it,
probes each rule here and writes the rules as drafts. A principle pack
imported the same way is installed under `whetstone/principles/`.
`--export-pack <file> --principles` writes the accepted rulebook's
principles in your own words as such a pack
(`whetstone.principle-pack.v1`, the shape pstack and `mattpocock` ship in),
named after the repository and versioned by the rulebook's commit; set its
license, groups and enforcer hints in the file. Principles picked from a
pack are left out, because they already have one.

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
wh explain rule.no-dbg      # or a finding's fingerprint, or a principle id
```

Locally, `wh check` uses the rulebook in the working tree, drafts included,
so a draft rule can be tried before it is accepted. `--ci` uses only the
committed rulebook and refuses a rulebook with uncommitted edits.

`--staged` checks each staged file whole, so a commit that touches a file
with an older must violation is blocked until that file is clean.

A plain check and pre-push also report the chain from principle to rule
(`data.coverage`: the principles no rule serves and the rules that serve
none). That line never changes the verdict and never appears in
`findings.json`.

A must finding fails the check; a should finding is reported and never
blocks. Unknown or unavailable evidence (a query that compiles for no
language, a missing tool, a timeout) is never a pass. Each failure names its
repair route and the command that rechecks it.

`--sweep` runs only the rules below must and prints a JSON array with one
finding per flag, one per line: `fingerprint` (sha256 of the rule, the
repository's origin and the subject, stable across runs), `subjectDigest`,
`class`, `priority`, `title`, `responsibility` (the rule), `principle` (the
principle the rule serves, when the rulebook names one) and, where
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

`wh dash` opens the rulebook as one page on localhost. `wh` serves it from
files built into the binary, and the page makes no third-party request.
From the top, the page shows:

- a one-line header: the project, the branch, the rulebook's revision, how
  many rules block and how many report, whether this tab can edit, and
  whether strict mode (`require_served`) is on;
- the pending changes, when the rulebook in the working tree differs from
  its last commit: a before and after per entry, with Accept and Withdraw;
- the health strip, one line per problem: gates or the CI check not
  installed (with the command that fixes it), an unreadable rulebook, rules
  that read nothing, source files no rule reads, and principles no rule
  serves or rules that serve none;
- the mission and the principles. Each principle lists the rules that serve
  it, as links that open them. A principle no rule serves is marked, says
  what kind of rule could serve it, and offers to write one: Add rule opens
  with Serves and the enforcer already set;
- the hard rules (must) and the soft rules (should). Each row says how the
  rule is checked, and a rule that serves no principle is marked. Opened, a
  row shows the principle the rule serves and lets you edit every field
  where it stands;
- the retired entries and the changelog, which lists every commit that
  touched the rulebook.

Add a rule lets you pick a starter detected in the repository, write a rule
yourself, or paste your guidelines (From your guidelines) to compile them
into drafts. Until the rulebook holds a principle, a button adopts the
starting set. Before a mission and a rule are committed, the page is a short
guided setup that ends in one commit. Every write is a `wh change` and stays
a draft until it is accepted. `--read-only` serves a page that cannot edit.

`wh dash --trail` prints the same log as a TSV trail (ts, phase, decision,
why, evidence, result). `wh dash --json` returns the rulebook's state, its
revision and the changelog for agents. Three flags narrow that changelog:
`--search <text>` (subject, body, author or hash), `--as-of
<YYYY-MM-DDTHH:MM:SSZ>` and `--page-size <1-200>`, default 100. When more
commits matched than were returned, `data.changelog_truncated` is true.
`wh dash --export <dir>` writes the accepted rulebook and its changelog as
one static page with no script. When `WHETSTONE_PAGES` is `on`, the CI
workflow publishes that page to GitHub Pages.

## Contributing

Read [AGENTS.md](AGENTS.md) before changing the project. It opens with the
document map, which says which file answers which question.
[planning/direction.md](planning/direction.md) records the decisions that
fix where Whetstone is going. The three gates in AGENTS.md run in the
pre-push hook and in CI and must pass before every push.
