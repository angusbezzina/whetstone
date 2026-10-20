# Agent instructions for Whetstone

This file is for an agent or a person working on Whetstone itself. What
Whetstone is, and how to use it, is stated once, at the top of
[README.md](README.md). Where it is going is fixed in
[planning/direction.md](planning/direction.md). Read both before planning
any work.

## Document map

| Question | Answered in |
| --- | --- |
| What is Whetstone, and how do I install and use it? | [README.md](README.md) |
| How do I work on this repository (gates, tracker, release)? | this file |
| What does Claude Code read first? | [CLAUDE.md](CLAUDE.md), a pointer to this file |
| Which decisions are fixed, and what was removed for good? | [planning/direction.md](planning/direction.md) |
| What does Whetstone own, what do hosts own, and what are the contracts between them? | [planning/skill-cli-boundary.md](planning/skill-cli-boundary.md) |
| Who uses the dashboard, what is on it, and what must it never do? | [PRODUCT.md](PRODUCT.md) |
| What does the dashboard look like? | [DESIGN.md](DESIGN.md) |
| What changed, and what is not released yet? | [CHANGELOG.md](CHANGELOG.md) |
| What exact shape do the file, the envelope, the findings and a principle pack have? | [references/](references) (`*.schema.json`) |
| Which enforcer fits a concern? | [references/signal-strategies.md](references/signal-strategies.md) |
| What is a script enforcer's contract? | `whetstone/checks/README.md` in a wired repository, written from `src/scripts.rs` |
| What is open now? | `bd ready` |

## Where the product lives in the code

The README describes the behaviour. This section says where each part is
and what must not move.

- **The file.** `src/rulebook.rs` reads and writes `whetstone/rulebook.json`
  ([schema](references/rulebook.schema.json)). The reader is strict and the
  writer keeps entries sorted by id. A revision is the commit that last
  touched the file. A rule's digest is the sha256 of its canonical JSON.
- **Rules.** `src/domain/rule.rs` defines the two strengths and the five
  enforcers. `Rulebook::coverage()` in `src/rulebook.rs` is the one
  computation of the chain from principle to rule; `wh check`, `wh dash`,
  `wh explain` and `wh change` all read it. Every rule `wh change` writes is probed first (`src/probe.rs`),
  including those from onboarding, prose compilation and pack imports.
- **Gates.** `src/wire.rs` installs the pre-commit hook (`wh check --staged`,
  AST and script rules) and the pre-push hook (`wh check` for the pushed
  commits). It also scaffolds `assets/ci/whetstone.yml`, which runs the
  required status `whetstone/policy` as
  `wh check --ci --base … --format annotations`.
- **Findings.** `src/service/check/findings.rs` writes `findings.json`
  ([schema](references/findings-v1.schema.json)): `fingerprint`,
  `subjectDigest`, `class`, `priority`, `title`, `responsibility`, and when
  present `principle`, `unsure`, `sites` (a batched rule's finding) and
  `fix`. The shape is a contract: add fields, never change one.
- **The envelope.** `src/service/mod.rs` builds
  `whetstone.command-response.v1`
  ([schema](references/command-response-v1.schema.json)) for every command.
- **The agent contract.** `src/contract.rs` is the block `wh init --action
  wire` writes between the `whetstone:agent-contract` markers in a
  repository's `AGENTS.md`. Keep it under 25 lines and true to the commands.
- **Packs.** The principle packs pstack and mattpocock are
  `assets/pstack/principles.json` and `assets/mattpocock/principles.json`,
  loaded by `src/catalogue.rs`. Rule packs, including the built-in rust,
  typescript and python packs, live in `src/service/packs.rs`.
- **Hosts.** `action.yml` is the reusable Action. Its hidden runner is
  `wh act` (`src/act.rs`). The Action is a host like any other: it uses only
  the three contracts in
  [planning/skill-cli-boundary.md](planning/skill-cli-boundary.md).

The public surface is exactly `wh init`, `wh dash`, `wh change`, `wh check`
and `wh explain`. New behaviour goes into these five, never into a new
top-level command. `wh act` is not for people.

## How this repository checks itself

Whetstone is wired by hand, not by `wh init --action wire`. There are three
reasons:

- **The gates are wider than the wired hook.** They run clippy, the tests
  and the build. `.githooks/pre-push` runs all three gates below.
  `scripts/self-check.sh` is the third gate: it runs `wh check --ci` against
  Whetstone's own `whetstone/rulebook.json`.
- **This file is the agent contract here.** A wired contract block would
  repeat less than this file says.
- **There is no pre-commit hook.** Pre-push and CI are the gates.

The rulebook holds a mission and three must rules. It holds no principles,
so its rules name none they serve. Changing any of this is the owner's
decision. Record it here when it changes.

## Product invariants

The fixed decisions in [planning/direction.md](planning/direction.md) hold
in every change. Three more that the code keeps:

- The JSON envelope and the dashboard describe the same file: both read the
  typed projections in `src/projection/`, and the page invents no state.
- A rule changes only by an edit to the file that a person commits.
- A check's evidence goes under `.git/whetstone/` and is never a record.

## High confidence or silence

- Prefer five trusted rules to fifty noisy ones.
- Use native linters and formatters when they express the concern.
- Use AST rules only for type-independent structure that native tools miss
  ([signal strategies](references/signal-strategies.md)).
- Raw regex is not an enforcement strategy.
- Cite current primary documentation and state when a capability is unknown.
- Never claim a green check proves more than the rules it ran.

## Do not restore

What was removed for good is listed under "Removed from direction" in
[planning/direction.md](planning/direction.md), and the Python tooling went
with it. Do not restore old modules, commands, fixtures or compatibility
paths as shortcuts. The last revision of the earlier product is
`1b7fd8c341b8a5aaea742c564092fbcca26b51bb`; inspect it read-only. Preserve
`whetstone/rulebook.json`, `.beads/` and `.git/info/exclude`.

## Issue tracking

Whetstone's own work is tracked in Beads with `bd`. Beads holds tasks only;
it is not part of the product.

```bash
bd ready
bd show <id>
bd update <id> --status in_progress
bd close <id>
bd dolt pull
bd dolt push
```

If local Beads state is broken, start with `bd doctor` and the
non-destructive `bd bootstrap`. Never put task ids in code, comments, tests,
commit messages or pull request text.

## Design work

The dashboard follows [DESIGN.md](DESIGN.md) and [PRODUCT.md](PRODUCT.md).
Its source is `web/` (React, shadcn, Tailwind, Anime.js, Oswald, Vollkorn
and JetBrains Mono); `cd web && npm ci && npm run build` writes
`assets/dashboard/`, which `src/dashboard.rs` embeds. Commit the build with
the source change: no gate runs Node. The page makes no third-party
request, and its HTML, CSS and JavaScript stay under 600 KiB.

Design changes go through the impeccable skill, which reads PRODUCT.md and
DESIGN.md. The skill is not part of the repository. Where the owner has
installed it, it sits under `.claude/skills/` and `.agents/skills/` in the
main checkout, and its working files go in `.impeccable/`. A fresh clone or
worktree has none of these, and none of them is committed. A checkout may also hold `whetstone/.personal/`, left behind
by the earlier product. `wh` never reads it; leave it uncommitted.

## Research

Before technical research, get the current date. Prefer current-year and
prior-year primary sources, official docs, changelogs and migration guides.
Verify that an API or configuration exists in the dependency version in use,
and flag stale material.

## Rules and source changes

Whetstone's own rules are in its committed `whetstone/rulebook.json`,
changed with `wh change` and committed like any file. The check fails closed
for a malformed rulebook, an unusable AST query, unavailable required
tooling and a vacuous gate.

Preserve unrelated work. Do not hide failures by weakening tests, deleting a
required gate or bypassing hooks. Comments say what the code does now; delete
dead code rather than gating it.

## Required gates before every push

CI and the pre-push hook run these exact three gates:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --no-fail-fast && cargo test --doc
cargo build && scripts/self-check.sh target/debug/whetstone
```

The self-check is Whetstone checking itself: `wh check --ci` over the whole
tree against the must rules in Whetstone's own `whetstone/rulebook.json`
(rustfmt, clippy's `dead_code`, and an AST rule against `dbg!`, `todo!` and
`unimplemented!`). It fails on any must finding or unknown evidence, and
when no must rule ran. It needs `jq`. The tests run under `cargo-nextest`
(`cargo install cargo-nextest --locked`); `cargo test --doc` covers doc
tests. No gate needs `bd`, Python or Node. The hook prints each step's
seconds and the total. The budget is 2 minutes for the hook on a warm cache
and 10 minutes for CI's quality job; when a run goes over, find the slow
step and cut it rather than loosening the budget or skipping a gate.

At the start of a session that may push, make sure the hook is active:

```bash
test "$(git config core.hooksPath)" = ".githooks" || git config core.hooksPath .githooks
test -x .githooks/pre-push || chmod +x .githooks/pre-push
```

Never use `--no-verify`.

## Session completion

Before ending a work session:

1. File Beads issues for remaining work and update current statuses.
2. Run the three gates for code changes and any task-specific checks.
3. Commit only intended changes.
4. Run `git pull --rebase`, `git push`, and `bd dolt push`.
5. Verify `git status` is clean and up to date with origin.
6. Report the change, the automated evidence, any manual evidence, the
   tracker state and what was published, separately.

Work is not complete until the push succeeds. A session that changed no
file and no issue has nothing to commit or push; it reports what it found.

## Release protocol

Before a release, pass all gates, update `CHANGELOG.md`, set the matching
version in `Cargo.toml`, commit `chore: release vX.Y.Z`, tag the same
version, push commit and tag, wait for `release.yml`, inspect the release,
and test the installer. Never force-push or delete a published tag, and
never tag a version that differs from `Cargo.toml`.
