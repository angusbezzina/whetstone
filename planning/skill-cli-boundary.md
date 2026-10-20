# Whetstone and its hosts: the boundary

Whetstone is a repository's rulebook and the check that enforces it
([README.md](../README.md) states the product). Hosts are whatever acts on
it: an agent session, a GitHub Action, a controller
such as Virgil's Messo. This file says what each side owns and the three
contracts between them. Keeping the line sharp keeps the `wh` binary small
and lets any host use it without code written for that host.

## Who owns what

| Side | Owns | Lives in |
| --- | --- | --- |
| The repository | The rulebook, its history and its review | `whetstone/rulebook.json`, Git, the platform's branch protection |
| Whetstone | Reading and checking the rulebook, running the rules, the findings, the envelope, the dashboard | The `wh` binary |
| Hosts | Memory, tasks, the agent, what to do with a finding | The host |

Whetstone keeps no store, no account, no network service and no
host-specific code. A check's evidence (command output) is written under
`.git/whetstone/` and is never a record.

## The three contracts

### The rulebook file

`whetstone/rulebook.json`, schema `whetstone.rulebook.v1`
([schema](../references/rulebook.schema.json)). It holds:

- `mission`: one statement;
- `principles`: each with an id, a statement and a source, either a
  principle from a pack (pstack's or the one distilled from Matt Pocock's
  skills, both compiled into `wh`, or a repository's own, each pinned to a
  version) or the owner's own;
- `rules`: each with an id, a statement, a rationale, one `strength`
  (`must`, `should`), one `enforcer`, optional labelled `examples`, a
  `source`, optional `paths`, and the sweep fields `sweep`,
  `sweep_priority` and `sweep_unsure`;
- `retired`: the ids taken out of the rulebook, each with its reason;
- `require_served` (optional, default false): strict mode, under which
  `wh change --accept` refuses a rulebook in which a principle has no rule
  serving it or a rule serves no principle.

The reader is strict: an unknown field, a duplicate id, an empty statement,
an unknown catalogue principle or a path that leaves the repository is an
error naming the field. The writer keeps a stable layout (entries sorted by
id) so a diff shows only what changed. A revision is the commit that last
touched the file; a rule's digest is the sha256 of its canonical JSON. A
host reads the file directly; it never needs `wh` to do so.

### findings.json

`wh check --sweep` runs the rules below must and prints a JSON array, one
finding per flag and one per line:

| Field | Meaning |
| --- | --- |
| `fingerprint` | sha256 of rule, repository origin and subject; stable across runs, repeats in one file numbered in source order |
| `subjectDigest` | digest of the flagged content, so a host sees when it changed |
| `class` | the rule's `sweep`: `discuss`, `confirm_first` or `draft_pr` |
| `priority` | the rule's `sweep_priority`, 0 (most urgent) to 4 |
| `title` | the rule's statement and where it was flagged |
| `responsibility` | the rule id |
| `principle` | present when the rulebook names the principle the rule serves: its id |
| `unsure` | present when the rule's `sweep_unsure` covers the flag: the reason |
| `fix` | present when the rule's formatter or linter can rewrite the file: the tool's own diff, which applies with `git apply` (the check never applies it) |

The class bounds a host's reach: `draft_pr` is a mechanical fix the host
may open as a draft pull request, `confirm_first` needs a person to approve
the exact plan first, and `discuss` only posts and asks. Only a
person widens a class, by changing the rule. The shape is a contract; new
fields are additive and existing ones never change meaning.

### The JSON envelope

Every command with `--json` returns `whetstone.command-response.v1`
([schema](../references/command-response-v1.schema.json)): `state`
(`success`, `violated`, `unknown`, `unavailable`, `needs_input`,
`needs_decision`, `stale`, `conflict`), `summary`, `evidence`,
`blocking_questions`, `permitted_actions` and `data`. A request with
`--json` never prompts. A failed check's findings each carry a repair route
and the exact `wh check --rule <id>` that rechecks them, so an agent can
repair and recheck in a loop without knowing anything else about Whetstone.

## Rules and enforcers

Every enforcer is mechanical:

- `ast`: a tree-sitter query with an optional `language` (Rust, TypeScript,
  Python); every node captured as `@match` is a violation;
- `lint`: a linter command and the code it must not report;
- `formatter`: a formatter's check command;
- `test`: a test command that must pass;
- `validator`: any command whose exit status is the verdict.

`must` blocks the gate. `should` reports, never blocks, and is what the
sweep runs. How to choose an enforcer is in
[signal-strategies.md](../references/signal-strategies.md).

Rule execution is deliberately narrow: no shell (literal argv, `&&` chains
only), an allowlisted environment, a repository-scoped working directory, and
bounded time and output. A timeout, truncation, missing tool or a query that
compiles for no language is unknown, never a pass. A check where no must rule
ran is not green.

## Gates and team enforcement

Every tool commits and pushes through Git, so the gates sit there.
`wh init --action wire --hooks` installs them, chaining any existing hook,
and `--ci` scaffolds the workflow:

1. **Pre-commit**: `wh check --staged` runs the AST and script rules on the
   staged index only.
2. **Pre-push**: `wh check` for the commits being pushed, against the
   remote's base.
3. **CI**: the required status `whetstone/policy`, run as
   `wh check --ci --base <base> --format annotations` so each flag appears
   as a GitHub annotation beside the code. It enforces the committed
   rulebook and refuses an uncommitted edit to it. On a push to main, the
   scaffold (`assets/ci/whetstone.yml`) also runs the sweep and uploads
   `findings.json` as the `scan-findings` artifact.

The pre-push hook checks exactly what is pushed: it refuses (unknown, with
the reason) when the pushed commit is not checked out or tracked files have
uncommitted changes, so a push of another branch, several refs at once or a
tag on an older commit is refused rather than vouched for. Untracked files
are not part of a push and never count. Command enforcers still run in the
working tree, where an untracked helper can influence them; CI, on a clean
checkout, is the backstop.

Team review of rule changes is the platform's own branch protection
(required review, the required status, no force pushes) with CODEOWNERS on
the rulebook and the workflow. Whetstone keeps no second trust root.

## Changing the rulebook

`wh change` is the editor, and the dashboard calls it for every write:

- `--kind mission|principle|rule --record-id <id> --content ... --rationale
  ... --definition <json>` drafts an edit to the file;
- `--serves <principle id>` on a rule change names the principle the rule
  serves (one the rulebook holds) and writes it as the rule's source;
- `--retire <id> --rationale ...` drafts a retirement, and refuses a
  principle that rules still serve;
- `--require-served true|false` drafts strict mode on or off;
- `--accept [--rationale ...]` commits the file, and only it, with the edits
  and the reasons in the message;
- `--withdraw` restores the file's last commit;
- `--preview` shows the exact effect and writes nothing;
- `--from-text <text>` and `--from-file <file>` compile prose into probed
  drafts where the text names its enforcer, and return the rest as needing
  one;
- `--from-finding <fingerprint>` drafts a must rule from a finding that
  keeps coming back;
- `--import-pack <file, URL, or rust, typescript or python>` imports a rule
  pack's rules as probed drafts, or installs a principle pack under
  `whetstone/principles/`;
- `--export-pack <file>` writes the committed rules as a rule pack
  (`whetstone.rule-pack.v1`). With `--principles`, it writes the rulebook's
  own principles as a principle pack (`whetstone.principle-pack.v1`)
  instead.

Every rule a change adds or re-aims is probed first: its enforcer runs once
here, and a rule that cannot be checked is not written without
`--allow-unvalidated`.

`wh init --action agree` drafts the mission, principles and starter rules the
same way. The decision log is the file's Git history: `wh dash --trail`
prints it as a TSV trail (ts, phase, decision, why, evidence, result), the
format pstack's show-me-your-work audit reads, and the dashboard's Changelog
shows the same commits. `wh dash --json` returns that changelog to an
agent, narrowed with `--search`, `--as-of` and `--page-size`.

## What a host does

A host that wants to work a repository's rules:

1. reads `whetstone/rulebook.json` for the mission, principles and rules,
   and cites a rule by its path and commit;
2. runs `wh check --json` (or reads CI's `whetstone/policy`) and repairs
   each must finding with the recheck command it names. When it needs the
   why, it asks `wh explain <rule id or fingerprint>` for the rule, its
   rationale and the principle it serves;
3. runs or downloads the sweep's `findings.json` and acts on each finding
   within its class, keyed by `fingerprint` so a finding it has already
   worked is recognised on the next run;
4. proposes a rule change as an edit to the file in a pull request, never by
   writing anywhere else.

Memory, tasks, scheduling and the agent itself stay in the host.

Two hosts ship with Whetstone, and both use only the contracts above:

- **The agent contract.** `wh init --action wire` writes a short block into
  the repository's `AGENTS.md`. It tells any agent how to check, read the
  envelope, repair, explain a finding and propose a rule.
- **The reusable GitHub Action** (`action.yml`, which runs the hidden
  `wh act`). It works a sweep's `findings.json` by class:
  - a draft pull request per `draft_pr` finding, applying its `fix` when it
    has one;
  - an issue asking for approval per `confirm_first` finding;
  - an issue for discussion per `discuss` finding.

  Running it twice changes nothing new, because it keys each item on the
  finding's fingerprint and subject digest. The CI scaffold runs the Action
  once the repository variable `WHETSTONE_WORK` is `on`. Once
  `WHETSTONE_PAGES` is `on`, it also publishes the read-only rulebook page
  (`wh dash --export`) to GitHub Pages.
