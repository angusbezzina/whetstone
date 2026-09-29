# Agent instructions for Whetstone

Whetstone helps people delegate more to coding agents. It turns a project's
mission, the pstack principles the owner picks and exemplar codebases into a
short list of agreed rules; every agent and person, in any tool, is briefed on
them, checked against them, and told when to stop and ask.

## Current direction

The owner adopted the delegation plan on 2026-09-27. Beads epic
`whetstone-ppq` is the plan of record and supersedes `whetstone-k5r`. Read
`planning/direction.md` and the epic description before planning any work.

- **One strength, one enforcer per rule.** Strength is must, should or
  advisory. The enforcer is, in order of preference, a mechanical check, a
  literal yes/no Jev question (TypeSafe System One) or a review
  (`/interrogate` or a named person).
- **Jev lives in the driver, never the kernel.** It can flag but never pass a
  must rule. It reports `unavailable` without a network or `TYPESAFE_API_KEY`.
  Receipts pin the model and carry question and input digests. New Jev rules
  run in shadow mode until `wh eval` measures their precision. Rule and diff
  text may go to TypeSafe (owner decision 2026-09-27), with optional redaction
  and local-only rules. Keys never enter records, receipts or logs.
- **Gate where every tool passes.** Checks run at git pre-commit (fast,
  staged, mechanical), at pre-push (full `wh check`) and as a required CI
  status. The Claude Code Stop hook is only the fast repair loop.
- **Nothing learns on its own.** Demotions, promotions, hardening and imported
  `/reflect` edits are drafts the owner accepts.

Two 2026-09-10 decisions still stand:

- **Beads is the record store.** `src/beads.rs` is the typed layer over
  `bd --json`: one bead per record revision, the typed body as ASCII-escaped
  canonical JSON in metadata (digest verified on every read), lifecycle as a
  label. Drafts and receipts live in a private Beads database under
  `.git/whetstone/` that is initialized and used with Git discovery fenced off
  and must never have a remote; `wh push` and `wh pull` wrap `bd dolt push`
  and `bd dolt pull` on the repository's shared database. Keep the domain and
  agreement layers storage-agnostic.
- **pstack interop, not imitation.** Whetstone uses pstack's `how`, `why`,
  `blast-radius`, `interrogate`, `reflect` and `maintain-verification-skill`
  and never reimplements them. The generated `verify-<app>` skill stays a
  valid pstack verification skill (frontmatter plus exactly four H2s per
  feature file, a pstack-shaped `features/README.md`, `wh init --action
  import`, control-adapter driver vocabulary, show-me-your-work TSV).
  Whetstone owns the format, and pstack's layout is its projection.

Removed from direction; their code was deleted in `whetstone-ppq.2` (stored
records of these kinds are read as retired and never written):

- the eight-decision onboarding;
- values, key metrics and outcome observations;
- policy exceptions;
- scopes and environments;
- PR-manifest activation (`wh push --propose`, `wh change --activate`,
  `wh init --ci --reviewer`, `authority.json`);
- the repair-host socket protocol and `wh check` repair-session flags;
- mandates.

Do not extend these, and do not build new work on them.

The legacy dependency-extraction product was removed. Its last complete
revision is `1b7fd8c341b8a5aaea742c564092fbcca26b51bb`; use it only for
read-only archaeology or recovery. Do not restore old modules, commands, TUI,
packs, generators, MCP server, compatibility aliases, Python runtime,
fixtures, or packaging as shortcuts. Preserve project data under `whetstone/`,
`.beads/` and `.git/info/exclude`. Deleting legacy files under `whetstone/`
needs the owner's explicit yes on task `whetstone-ppq.2.6`.

The public workflow families are exactly `wh init`, `wh dash`, `wh change`,
`wh check`, `wh pull`, and `wh push`, all implemented against the Beads store.
New behaviour goes into these families, never new top-level commands. Do not
describe the product as accepted until the milestone exits in `whetstone-ppq`
are recorded. Bare `wh` is honest read-only orientation. The hidden developer
gates are `validate`, `eval`, and `scan`.

Authoritative references:

- `planning/direction.md`: the plan, fixed decisions and evidence targets.
- `planning/skill-cli-boundary.md`: skill, driver and kernel boundary, storage
  and the pstack interop contract.
- `planning/direction-demo/index.html`: the dashboard reference (visual world
  current; onboarding and Foundations content are replaced by
  `whetstone-ppq.1.3`); `planning/direction-demo/smoke.mjs` validates it.
- `PRODUCT.md` and `DESIGN.md`: product truth and the design tokens/rules the
  dashboard implements; `.impeccable/surfaces/` holds the surface brief.
- `references/signal-strategies.md`: how to choose a mechanical signal.
- `planning/archive/`: superseded ADRs (direct Dolt, PR-manifest trust
  boundary) and the R0 prune inventory, kept as history.

## Product invariants

- The skill owns judgment; typed services own deterministic, replayable work.
- Stable JSON and human views describe the same source records.
- Guidance, checks, evidence, and accountable verdicts remain distinct.
- Every rule has exactly one strength and one enforcer; a judgment answer never passes a must rule.
- Failures return to the same worker for repair and recheck.
- Unknown, stale, skipped, or unavailable required evidence is never success.
- Local and private records stay private until explicitly selected for sharing.
- Proposal, acceptance, check receipt, and verdict are separate events.
- Rule changes, including demotions and promotions, are drafts a person accepts.
- Team review uses the platform's branch protection and a required `wh check` status.
- Consequential decisions are append-only and inspectable from project start.
- Existing trackers and native repository controls remain authoritative.

## High confidence or silence

- Prefer five trusted safeguards to fifty noisy rules.
- Use native linters/formatters when they express the concern.
- Use AST checks only for type-independent structure that native tools miss.
- Raw regex is not an enforcement strategy.
- Put taste, semantics, and type-aware advice in guidance or accountable review.
- Cite current primary documentation and state when a capability is unknown.
- Never claim prompt delivery, a green check, or a demo proves enforcement.

## Issue tracking with Beads

Use `bd` for all planning and issue tracking unless the user explicitly says
otherwise. Do not substitute TODO comments or ad-hoc planning files.

```bash
bd ready
bd show <id>
bd update <id> --status in_progress
bd close <id>
bd dolt pull
bd dolt push
```

Use current Dolt-native collaboration, never legacy `bd sync` or a
`beads-sync` branch. If local state is broken or another machine cannot see
issues, start with `bd doctor` and the non-destructive `bd bootstrap` (bd 1.3).
`./scripts/beads-repair.sh` is the last resort: it backs up and re-clones the
local Dolt database from the remote.

Beads is also Whetstone's record store (see the current boundary). Whetstone
records in Beads are `record`, `decision` and `receipt` beads labelled
`whetstone` whose JSON metadata is validated on every read; never hand-edit
their metadata with `bd update`, and never prune, compact or delete them to
tidy the tracker. Tests use throwaway `bd init` databases only; never point a
test or experiment at this repository's `.beads`.

Install requirements for Whetstone itself: the `wh` binary, `bd` 1.1.2 or
later, Node 22 for the verification driver, and Chrome or Chromium for web
surfaces. No `dolt` binary.

## Design work

UI changes use the `impeccable` skill. It is installed project-locally under
`.agents/skills/impeccable` (ignored by Git, pinned by `skills-lock.json`);
reinstall with `npx skills add pbakaus/impeccable -y`. Run its `context`
command once per session before editing UI, follow `DESIGN.md`, and update the
direction reference before the product when the information architecture
changes.

## Research

Before technical research, get the current date. Prefer current-year and prior-
year primary sources, official docs, `llms.txt`, changelogs, and migration
guides. Verify that an API or configuration exists in the dependency version in
use, and explicitly flag stale material.

## Rules and source changes

Do not hand-edit `whetstone/rules/**`; those files are preserved user data until
task `whetstone-ppq.2.6` migrates them into Beads rules. The retained verifier must fail closed for malformed project rules,
missing AST queries, unsafe validator paths, unavailable required tooling, and
non-vacuous gate failures.

Use `apply_patch` for source and documentation edits. Preserve unrelated work.
Do not hide failures by weakening tests, deleting a required gate, or bypassing
hooks.

## Required gates before every push

CI and the pre-push hook must run these exact eight gates meaningfully:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
python3 -m ruff format --check scripts/ tests/
cargo run --quiet --release -- validate
cargo run --quiet --release -- eval
cargo run --quiet --release -- scan src --lang rust --json --no-fail
python3 -m pytest -q
```

The scan gate additionally requires `files_scanned > 0`, `rules_applied > 0`,
`config_issues_count == 0`, and `violations_count == 0`. Eval must exercise at
least one rule and one scanner-backed golden example. Zero work is not green.

At the start of a session that may push, ensure the hook is active:

```bash
test "$(git config core.hooksPath)" = ".githooks" || git config core.hooksPath .githooks
test -x .githooks/pre-push || chmod +x .githooks/pre-push
```

Never use `--no-verify`.

## Session completion

Before ending a work session:

1. File Beads issues for remaining work and update current statuses.
2. Run the eight gates for code changes and any task-specific proofs.
3. Commit only intended changes.
4. Run `git pull --rebase`, `git push`, and `bd dolt push`.
5. Verify `git status` is clean and up to date with origin.
6. Report implementation, automated evidence, human smoke evidence, tracker state, and publication separately.

Work is not complete until push succeeds. Do not say it is merely ready to push.

## Release protocol

Before a release, pass all gates, update `CHANGELOG.md`, set the matching version
in `Cargo.toml`, commit `chore: release vX.Y.Z`, tag the same version, push commit
and tag, wait for `release.yml`, inspect the release, and test the installer.
Never force-push or delete a published tag, and never tag a version that differs
from `Cargo.toml`.
