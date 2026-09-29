# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This log starts with the rebuilt product. Release notes for the previous
product line (0.1.0 to 0.12.0) are in Git history: `git show 1b7fd8c:CHANGELOG.md`.

## [Unreleased]

Whetstone is rebuilt around six workflows and a verification skill that any
agent can use. Nothing here has been released yet.

**Direction change (2026-09-27).** The owner adopted the delegation plan
(`planning/direction.md`, Beads epic `whetstone-ppq`, which supersedes
`whetstone-k5r`). Rules get one strength and one enforcer: a mechanical check,
then a Jev question, then review. Checks move to git pre-commit, pre-push and
a required CI status, and onboarding becomes mission, pstack principles and
exemplar codebases. Several entries below describe code this plan removes:
the eight-decision agreement, key metrics, team activation through pull
requests, and gate exceptions. They stay here as a record of what was built.

### Added (delegation plan, `whetstone-ppq`)

- Rule v2 (`references/rule-v2.schema.json`): one strength and exactly one
  enforcer (mechanical, Jev question or review), labelled examples, source,
  hand-raise triggers and privacy. Earlier standards and guidance are read as
  rules.
- Onboarding: `wh init --action setup` (Beads and pstack detection, one
  confirmed install, `whetstone/tools.lock.json`), mission plus pstack
  principles plus three starter rules in `--action agree`, and
  `--action exemplar` to draft rules from a codebase the owner admires.
- Jev questions through the driver's `ask` command with kernel-side
  redaction and local-only rules; shadow mode, confidence bars, and the rule
  that a Jev answer never passes a must rule.
- Gates: git pre-commit (`wh check --staged`), pre-push
  (`wh check --base <remote>`) and a required CI status (`--ci`), chaining any
  existing hook; Codex and Cursor stop hooks next to Claude's.
- Review attestations (`wh check --attest`), briefs (`--brief`), raised hands
  as `bd human` issues (`--raise-hand`, answered with `wh change --answer`),
  flag labels (`--accept-flag`, `--dismiss-flag`), false-flag rates, tuning
  drafts (`wh change --tune`), hardening candidates (`--hardens`), and
  `wh change --migrate` for earlier values, philosophy and YAML rules.
- Proof quality: flaky proofs are quarantined, and `wh check --mutate` runs
  declared mutations in an isolated worktree to catch hollow proofs.
- `wh eval` scores each rule's examples by enforcer and checks the pstack
  round trip.
- The generated verification skill carries `rules/*.md` and
  `whetstone.verify.json` (`references/verify-skill-v1.schema.json`).

### Removed

- The eight-decision onboarding, core values, implementation philosophy and
  key metrics as live kinds, policy exceptions, scopes, PR-manifest activation,
  repair-host sockets and transport, and mandates. Their stored records are
  still read byte-exactly as retired kinds and can never be written.

### Fixed

- A second `wh change` (or `--accept`) without `--request-id` no longer
  collides with the first: the default request id is derived from the
  change's input.
- Pre-push checks exactly what is pushed: the hook passes the pushed commit,
  and the check refuses when that commit is not checked out or tracked files
  have uncommitted changes; untracked files are not part of the push.
- A review attestation binds to the tree it reviewed, so it stops holding as
  soon as the checked content changes (one more commit included).
- Committing Whetstone's own wiring, a change with no file an AST rule reads,
  a whitespace-only signature change, a new module with public items, or a
  change outside a path-scoped review no longer blocks a commit; neither
  does a repository whose rules all run at pre-push, or whose only rules are
  in shadow.
- Staged checks record no receipts, which brings pre-commit to about 1.5s.
- `wh init --action agree` replays exactly, resumes a half-written batch,
  accepts the mission already in force, and writes nothing when stale; batch
  writes work beside retired records.
- The Stop hook no longer claims a hand was raised when none could be filed.

### Documentation

- README, AGENTS, CLAUDE, PRODUCT, SKILL and the planning docs now describe
  the delegation plan, and separate what exists today from what is planned.
  The storage and trust-boundary ADRs and the R0 prune inventory moved to
  `planning/archive/` with superseded notes.

### Added

- **Six workflows** with one JSON envelope (`whetstone.command-response.v1`):
  `wh init`, `wh dash`, `wh change`, `wh check`, `wh pull` and `wh push`. Every
  workflow that writes accepts `--dry-run`.
- **A private project agreement.** `wh init` records the owner's eight
  decisions (mission, desired outcome, values, philosophy, owner, first
  safeguard and its scope, revision triggers, first gate) in a private store
  under `.git/whetstone/`. Nothing is committed or shared. Run by a person
  before any agreement exists, it opens the dashboard's guided onboarding and
  logs each recorded step to the terminal; `--no-open` prints the decisions
  and the exact command instead. Machine callers always get the JSON envelope.
- **A verification skill for any agent.** `wh init --action wire` generates a
  skill, feature map and driver from the accepted records and writes the same
  bytes into `.claude/skills`, `.cursor/skills` or `.agents/skills`. Feature
  files follow pstack's four-section format, so pstack's skills work on them
  unchanged. `wh init --action import` turns edited or pstack-generated skills
  back into drafts.
- **Proof, not claims.** `wh check` runs every gate and records receipts bound
  to their evidence. A pass without evidence is unknown, never green.
  - `--feature <id>` drives one feature; `--step` adds the change's own
    assertions to the receipt.
  - `--changed` proves what a change touched; `--base <revision>` includes
    committed work.
  - `--sweep` drives every feature and reports each as proven, failed,
    unreachable or skipped, with map hygiene findings.
- **A verification driver** (`whetstone/verify/drive.mjs`, Node 22, no
  dependencies) for web, CLI and HTTP apps: isolated launches, doctor checks,
  screenshots and cleanup that keeps evidence.
- **A dashboard** (`wh dash`) with four views: Dashboard (mission, what needs
  attention, metrics, gates), Foundations, Checks (every gate's last run and
  its evidence) and Changelog (searchable decision history). `wh dash --trail`
  exports the decision trail as TSV.
- **Agent hosts.** Claude Code gets hooks: session context at start and failed
  checks handed back to the same session at stop, at most three times. Other
  agents check in with `wh check --changed --host <name>`.
- **Team sync.** `wh push` shares an exact, confirmed package, and private
  drafts never leave the machine. `wh pull` never runs anything it receives.
- **Team activation.** A shared rule becomes binding only through a pull
  request approved by a different account on a protected branch. CI enforces
  activated rules with `wh check --required`, restores tampered checkers from
  their pinned versions, and reports reviewed, expiring exceptions as excepted,
  never passing.
- **Maintenance.** Map drift and hygiene show up as attention items, and
  `wh check --maintain-outcome` records the result of a maintain pass.
- **A design system.** The dashboard follows the Whetstone logo: black and
  orange, white and orange in light mode, with ShadCN token names
  (`--background`, `--foreground`, `--muted`, `--primary`, `--ring`, ...) in
  the dashboard's own CSS. Every option is present as a quiet ghost and the
  one live thing is struck in orange; states are marks, not hues (a lit disc
  for pass, an orange cross for fail, a ring for unknown or stale, a dashed
  ring for draft). No boxes or rules; the tokens and rules are in `DESIGN.md`.

### Changed

- Records are stored in Beads (`bd` 1.1.2 or later) instead of Whetstone's own
  Dolt repository. The `dolt` binary is no longer needed.

### Removed

- The previous product line: its terminal UI, packs, legacy commands, Python
  runtime and generated artifacts. See the Git history above for its notes.
