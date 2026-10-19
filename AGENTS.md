# Agent instructions for Whetstone

Whetstone is a repository's rulebook: enforced rules with a rationale and an
agent protocol, between `AGENTS.md` (prose, unenforced) and a linter
(enforced, no rationale). Read [README.md](README.md) for the commands and
[planning/direction.md](planning/direction.md) for the fixed decisions before
planning any work.

## The product in one screen

- **The rulebook is a file.** `whetstone/rulebook.json`
  (`whetstone.rulebook.v1`, [schema](references/rulebook.schema.json)) holds
  the mission, the principles (from a principle pack: pstack and one
  distilled from Matt Pocock's skills are compiled into `wh`, a repository
  adds packs under `whetstone/principles/`, and `--principle starting` picks
  the set the packs offer as a starting point; or the owner's own) and the
  rules. A revision is the commit that last touched the
  file, a rule's digest is the sha256 of its canonical JSON, the decision log
  is Git history, a draft is an uncommitted edit, and a rule change on a team
  is a pull request.
- **One strength, one enforcer per rule.** `must` blocks the gate; `should`
  reports, never blocks, and becomes a sweep finding. The enforcer is
  mechanical: `ast`, `lint`, `formatter`, `test` or `validator`. A
  `validator` or `test` command that names a script under `whetstone/checks/`
  is a script enforcer (contract in `whetstone/checks/README.md`).
- **Probed on every write.** Every rule `wh change` writes (onboarding, prose
  compilation and pack imports included) has its enforcer run once over the
  repository first; a rule that cannot be checked here is not written
  without `--allow-unvalidated`.
- **Sweep fields.** `sweep` (`discuss`, `confirm_first`, `draft_pr`; absent
  is discuss), `sweep_priority` (0 to 4, default 2) and `sweep_unsure`
  (paths or a pattern, with a reason).
- **Gates.** Pre-commit runs `wh check --staged` (AST and script rules on
  the staged index), pre-push runs `wh check` for the pushed commits, and CI
  runs the required status `whetstone/policy` (`wh check --ci --base
  --format annotations`, so each flag shows beside the code).
- **Findings.** `wh check --sweep` prints `findings.json`: one finding per
  flag with `fingerprint`, `subjectDigest`, `class`, `priority`, `title`,
  `responsibility`, `unsure` and, for a tool that can rewrite files, `fix`
  ([schema](references/findings-v1.schema.json)). Its shape is a contract;
  do not change it. `wh explain <fingerprint>` resolves a finding to its
  rule and the commits that accepted it.
- **Hosts.** A host (Virgil's Messo, a GitHub Action, an agent session)
  consumes three things: the rulebook file, `findings.json` and the
  `whetstone.command-response.v1` envelope. Whetstone keeps no store and no
  host-specific code; hosts own memory, tasks and the agent. The reusable
  Action (`action.yml`) is one such host: it works `findings.json` by class
  with `gh`.
- **The agent contract.** `wh init --action wire` writes a short block into
  the repository's `AGENTS.md` between `whetstone:agent-contract` markers
  (`src/contract.rs`); text outside them is the team's. Keep it under 25
  lines and true to the commands.
- **Packs.** A principle pack (`whetstone.principle-pack.v1`) lists
  principles; a rule pack (`whetstone.rule-pack.v1`, `wh change
  --export-pack` / `--import-pack`) carries accepted rules with a digest and
  arrives as drafts.

The public surface is exactly `wh init`, `wh dash`, `wh change`, `wh check`
and `wh explain`. New behaviour goes into these five, never into a new
top-level command. Bare `wh` is read-only orientation. The one hidden
command is `wh act`, the runner the reusable Action calls; it is not for
people.

## Product invariants

- Every rule has exactly one strength and one mechanical enforcer.
- Unknown, stale, skipped or unavailable evidence is never success, and a
  check where no rule ran is not green.
- The JSON envelope and the dashboard describe the same file.
- A rule change is an edit to the file that a person commits; nothing changes
  a rule on its own.
- Team review is the platform's branch protection plus the required
  `whetstone/policy` status; Whetstone keeps no second trust root.
- Whetstone keeps no store: the rulebook file and Git are its only state. A
  check's evidence goes under `.git/whetstone/` and is never a record.

## High confidence or silence

- Prefer five trusted rules to fifty noisy ones.
- Use native linters and formatters when they express the concern.
- Use AST rules only for type-independent structure that native tools miss
  ([signal strategies](references/signal-strategies.md)).
- Raw regex is not an enforcement strategy.
- Cite current primary documentation and state when a capability is unknown.
- Never claim a green check proves more than the rules it ran.

## Do not restore

Removed paths stay removed: the record store and every adapter for it, team
sync commands, the hidden developer commands, the rule-file scanner, agent
host hooks, judgment enforcers and their receipts, the generated verification
skill, memory records and the Python tooling. The last revision of the
earlier product is `1b7fd8c341b8a5aaea742c564092fbcca26b51bb`; inspect it
read-only. Do not restore old modules, commands, fixtures or compatibility
paths as shortcuts. Preserve `whetstone/rulebook.json`, `.beads/` and
`.git/info/exclude`.

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

The dashboard follows [DESIGN.md](DESIGN.md) and [PRODUCT.md](PRODUCT.md):
dependency-free HTML, CSS and JavaScript in `assets/dashboard/`, startup
assets under 24 KiB, no third-party requests.

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

Work is not complete until the push succeeds.

## Release protocol

Before a release, pass all gates, update `CHANGELOG.md`, set the matching
version in `Cargo.toml`, commit `chore: release vX.Y.Z`, tag the same
version, push commit and tag, wait for `release.yml`, inspect the release,
and test the installer. Never force-push or delete a published tag, and
never tag a version that differs from `Cargo.toml`.
