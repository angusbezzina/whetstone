# Direction: rules as files

Decided by the owner on 2026-10-19. This file records the direction and the
fixed decisions. [README.md](../README.md) states the product and how to use
it, and the document map at the top of [AGENTS.md](../AGENTS.md) says where
everything else is. This file is not a backlog: tasks, order and acceptance
live in the project's tracker.

## Goal

A team that delegates work to coding agents needs rules that hold whatever tool
writes the code. Prose in `AGENTS.md` is read but never enforced; a linter
is enforced but gives no reason and nothing an agent can act on. Whetstone
is the piece between them: enforced rules, each with a rationale, and an
agent protocol any host can consume. The solution has to be, in priority
order, simple, customisable, versatile, strong and scalable.

## The plan

1. **Write the rulebook once.** `wh init` drafts the mission, the principles
   the owner picks from the catalogue (or writes) and the starter rules the
   repository can hold into `whetstone/rulebook.json`. The owner commits it
   with `wh change --accept`.
2. **Enforce every rule mechanically.** Each rule has one strength and one
   enforcer:
   - `must` blocks the gate; `should` reports, never blocks, and becomes a
     sweep finding;
   - the enforcer is `ast` (a tree-sitter query), `lint` (a linter and the
     code it reports), `formatter`, `test` or `validator` (a command).
3. **Gate where every tool passes.** Every tool commits and pushes through
   Git, so the gates sit there:
   - pre-commit: `wh check --staged`, AST and script rules on the staged
     index;
   - pre-push: `wh check` for the commits being pushed;
   - CI: the required status `whetstone/policy`, `wh check --ci --base`,
     which reads only the committed rulebook.
4. **Give findings to whatever works them.** `wh check --sweep` runs the
   rules below must and prints `findings.json`. A host decides what to do
   with each finding, within the reach the rule's sweep class allows.

## Fixed decisions

### Public surface
Five commands: `wh init`, `wh dash`, `wh change`, `wh check`, `wh explain`.
The one hidden command is `wh act`, the runner the reusable GitHub Action
calls. New behaviour goes into these five, never into new top-level
commands. Bare `wh` is read-only orientation; `--json` is the agent
interface.

### The rulebook is the store
`whetstone/rulebook.json` (`whetstone.rulebook.v1`) holds the mission, the
principles and the rules; retired entries are listed with their reasons.
Whetstone keeps no other store.
- A revision is the commit that last touched the file.
- A rule's digest is the sha256 of its canonical JSON.
- The decision log is the file's Git history (`wh dash --trail`, the
  dashboard's Changelog).
- A draft is an uncommitted edit; accepting is `wh change --accept`, which
  commits the file and only it; withdrawing restores its last commit.
- Team sync is Git, and a rule change on a team is a pull request.

### Rules
- One strength: `must` or `should`.
- One mechanical enforcer: `ast`, `lint`, `formatter`, `test`, `validator`.
- Optional `paths` scope the rule.
- Sweep fields: `sweep` (`discuss`, `confirm_first`, `draft_pr`; absent is
  discuss), `sweep_priority` (0 most urgent to 4; default 2) and
  `sweep_unsure` (paths or a pattern, with a reason). Only a person widens a
  sweep class.
- Unknown, stale, skipped or unavailable evidence is never a pass, and a
  check where no must rule ran is not green.

### Principles
No tool checks a principle, and a model never judges one. What is enforced
is the chain: every held principle is served by at least one rule (the
rule's source names it), and every rule serves a principle. When the chain
holds, the gates enforce the principles through the rules.
- `wh check`, `wh dash` and `wh explain <principle id>` report the gaps; a
  gap never changes a check's verdict and never enters `findings.json`.
- A principle that rules serve cannot be retired until they serve another.
- The optional `require_served` field (strict mode) makes
  `wh change --accept` refuse a rulebook with a gap.
- Principles carry no strength, priority or metric.

### Findings
The contract does not change shape. One finding per flag:
`fingerprint` (sha256 of rule, repository origin and subject, numbered in
source order for repeats), `subjectDigest`, `class`, `priority`, `title`,
`responsibility`, `principle` (the id of the principle the rule serves,
when the rulebook names one) and, where `sweep_unsure` covers the flag,
`unsure` with its reason.

### Hosts
A host is anything that acts on the rulebook: Virgil's Messo, a GitHub
Action, an agent session. It consumes the rulebook file, `findings.json` and
the `whetstone.command-response.v1` envelope. Whetstone contains no
host-specific code. Hosts own memory, tasks and the agent; a host that keeps
notes about a rule cites it by its path and commit.

### Team review
Team review uses the platform's own branch protection plus the required
`whetstone/policy` status. Protect `whetstone/rulebook.json` and the CI
workflow with CODEOWNERS. Whetstone keeps no second trust root, activation
state or authority file.

## Removed from direction

These are out of the product and must not come back:

- a record store of any kind, its adapters and its sync commands;
- judgment enforcers (model questions and reviews) and their receipts;
- the generated verification skill and its app automation;
- memory records and requests for help raised by agents;
- agent-host hook loops (the Git hooks are the loop every tool passes
  through);
- the rule-file scanner and its hidden developer commands;
- the eight-decision onboarding, values and metrics, exceptions, scopes,
  pull-request manifest activation and mandates.

## Evidence that it works

| Measure | Target |
| --- | --- |
| Time to a first committed rulebook | Under five minutes on a clean repo |
| Seeded violations missed | Zero on must rules |
| False flags | Under one per ten commits per rule |
| Consistency across tools | Same verdict on one change whatever agent wrote it |

## Authoritative references

- [skill-cli-boundary.md](skill-cli-boundary.md): what Whetstone owns and
  what hosts own, the gates and the contracts between them.
- [../references/rulebook.schema.json](../references/rulebook.schema.json)
  and
  [../references/command-response-v1.schema.json](../references/command-response-v1.schema.json):
  the file and the envelope.
- [../PRODUCT.md](../PRODUCT.md) and [../DESIGN.md](../DESIGN.md): product
  truth and the dashboard's design.
