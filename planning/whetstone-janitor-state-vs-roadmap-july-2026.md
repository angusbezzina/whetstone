# Whetstone + Janitor — State vs. Roadmap (July 2026)

> **Audience: the next agent (or human) picking up either project.** Everything
> here is verifiable — each section lists the commands that prove its claims.
> Written 2026-07-11. Facts go stale; the verify commands don't.
>
> Repos: `~/Development/whetstone` (github.com/angusbezzina/whetstone) ·
> `~/Development/janitor` (github.com/angusbezzina/janitor).
> Both use **beads** (`bd ready` / `bd show <id>`) for ALL tracking — start there.

---

## 1. The product thesis (why these exist)

**Owner's goal, verbatim:** *"keep my agents on task and writing code that I
feel is best practice"* — for both human and agentic users.

- **Whetstone** is the **standards layer**: it derives explicit, doc-cited
  coding rules from dependency documentation (judgment, once), then enforces
  them **deterministically forever** (tree-sitter scan, native lint configs,
  CI, an in-session hook, MCP) at zero marginal token cost. Two loops:
  1. *In-session enforcement* — a PostToolUse hook feeds violations back to the
     agent in the same turn.
  2. *Taste capture* — user preferences become durable rules (eval-verified) or
     guidance entries, portable across repos via packs.
- **Janitor** is the **night shift**: a cron-driven, forge-agnostic,
  agent-agnostic harness that turns Whetstone findings into tracked issues,
  dispatches a coding agent to fix each in an isolated worktree, **verifies the
  fix deterministically**, and opens small cited PRs + a morning digest.
  Thesis: *deterministic queue → agentic fix → deterministic verify.* Never
  merges. Knows when to stop.
- **Positioning** (researched, July 2026): the empty market square is
  *cron × any-forge × any-agent × deterministic cited queue*. Nearest neighbors:
  Renovate (the model, but deps-only), Sortie (executor plumbing, human-ticket
  queue), **shadcn/improve** (episodic judgment audits → plans for cheap
  executors; stateless by design). Our ratchet line vs improve: *"improve tells
  you what's wrong today; Whetstone makes sure it's never wrong again."* The
  2026 AI-PR-slop backlash (GitHub throttles, Godot ban) makes
  cited-findings-only the design that survives.

**Scope rulings (do not relitigate):** scope-discipline ("agent wandered
off-task", diff-vs-intent) is OUT of scope — Whetstone keeps agents
*on-standard*, not *on-scope* (`planning/skill-cli-boundary.md` §10). The TUI is
frozen EXCEPT onboarding + review (§11 amendment). Judgment lives in the skill,
determinism in the CLI — anything the TUI/wizard needs becomes a headless CLI
oracle first.

---

## 2. Where we ARE — Whetstone

Verify: `git log --oneline v0.10.0..HEAD` · `bd list --status=open` ·
`cargo run --release -- --help` · `wh status --setup --json`

### Shipped and RELEASED (v0.10.0, 2026-07-03)
- Epic `whetstone-4xw` (v0.9.1): skill-first reorientation — skill owns
  judgment, thin deterministic CLI.
- Epic `whetstone-5ox`: `wh eval` rule-quality bar (goldens through the real
  scanner + source-fidelity); trusted corpus `packs/<lang>/<dep>.yaml` (26 AST
  rules / 10 deps: fastapi, pydantic, sqlalchemy, httpx, react, next, zod,
  express, axum, serde); `wh mcp` (rules_query + scan); turnkey CI; dogfood
  gate; value-proof (`planning/measurements/value-proof.md`: 26/26 caught vs
  ruff 0).
- Epic `whetstone-z83`: in-session enforcement (`wh hook posttooluse` +
  installer), taste-guidance store (`whetstone/guidance/`), taste-capture
  workflow (SKILL.md), personal taste pack (`packs/templates/taste.yaml`),
  one-command agent onboarding (`wh init --claude`, corpus embedded in the
  binary via `src/corpus.rs`).
- Release infra: GitHub Releases (4 targets + checksums), `install.sh`,
  `wh update` self-update, Homebrew formula updated (tap NOT yet published).

### Shipped but UNRELEASED (15 commits on main past v0.10.0 — all of epic v5n)
Epic `whetstone-v5n` (closed 2026-07-05, validated by independent subagent:
round 1 found 5 issues → fixed → round 2 **APPROVE**):
- **Oracles** (CLI-first per boundary): read-only + injectable snapshot seam
  (`SnapshotOptions`, `resolve_merged_with` — previews flow the real merge
  seam, zero `.state` writes); `wh scan --with-pack` (preview + `from_candidate`
  tags + parity); `wh config conflicts` (same-id + formatter clashes,
  `references/conflicts-schema.md`); `wh status --setup` (derived checklist);
  `wh pack import` (shared import primitive); `wh onboard dismiss|reset`.
- **The onboarding wizard** (`src/tui/screens/onboard.rs`, bare `wh` on a TTY):
  Home (derived progress) → Express (≤3 keys) / Curated → Packs (live preview)
  → Sources (per-dep changelog-watch toggles) → Conflicts (deny resolution) →
  **Review, the gate** (nothing enforceable without confirm; citations +
  goldens shown; per-rule deny; approves agent-proposed candidates — the Infer
  return leg) → Payoff (first scan) → agent wiring. Zero business logic in the
  TUI; 6 headless wizard-logic tests.
- Resources-pack mechanism (`BundledPack.kind: starter|resource`) + first
  style-guide pack `packs/resources/airbnb-js.yaml`.
- Two-front-doors docs (wizard reviews pre-import; `init --claude` = delegated
  consent + prints "review anytime: run `wh`").

### Quality infrastructure (trust it; keep it green)
8-gate pre-push hook (`.githooks/pre-push`; mirrors CI; listed in CLAUDE.md):
clippy -D warnings · cargo test (~208) · ruff check · ruff format · `wh
validate` · `wh eval` · self-scan dogfood (must be 0) · pytest (182 parity
tests). **Never push without them; never `--no-verify`.**

### Open beads (8)
| Bead | P | What |
|---|---|---|
| `whetstone-b13` | **P1** | Publish Whetstone as installable agent skill (npx skills channel) — raised after the improve comparison; the one gap that threatens us |
| `whetstone-ykf` | P2 | The ratchet: audit-finding → rule capture bridge |
| `whetstone-4us` | P2 | Resources packs: expand wave (needs current-docs research) |
| `whetstone-6b4` (+nww/8ht/b7y) | P2 | Phase-2 epic: personas/archetypes — ARCHETYPES-ONLY v1; named personas need consent/legal |
| `whetstone-fed` | P3 | Leverage-ranked findings in `wh report` |

---

## 3. Where we ARE — Janitor

Verify (in `~/Development/janitor`): `git log --oneline` · `bd ready` ·
`cargo test` (102 tests) · `PLAN.md` (authoritative design + as-built notes)

- **V1 pipeline IMPLEMENTED** on `main` @ `99b7165` (SWEEP → TRIAGE → FILE →
  FIX → VERIFY → PROPOSE → REPORT; SQLite ledger; gh forge; gh-issues/none
  trackers; command-template agent seam with a live-verified `claude -p`
  preset; budgets; digest; pause/lockfile/branch guarantees). ~6.6k LOC,
  102 tests, 3 gates green. A **different agent implements this repo** —
  don't silently take over its in-flight work; `AGENTS.md` there carries the
  six non-negotiables.
- **Independent validation (2026-07-04): SOLID-WITH-GAPS.** Trust envelope
  held under sabotage (VERIFY fail-closed, ledger idempotency, single guarded
  push site, PROPOSE unreachable without verify). But **two majors would break
  the first real night**:
  - `janitor-07i` (P0): `gh pr create --base origin/main` — real gh rejects it.
  - `janitor-3u5` (P0): PR cap is effectively *lifetime* (no forge
    reconciliation) — nights ≥2 open zero PRs.
- **Open work:** hardening epic `janitor-7g6` — 7 children ALL OPEN (07i, 3u5,
  4xy budget-per-night, 1ea agent-failure surfacing, 85u controls polish, qdv
  digest/SWEEP hygiene, zu1 PR-update path) + `janitor-2wh` (leverage triage +
  verify auto-detect, from the improve comparison). Exit test `janitor-766`
  in_progress: **≥3 clean wall-clock nights on a real repo, 0 spam** — blocked
  on the two majors; readiness otherwise cleared (wh 0.10.0 ✓, gh authed ✓,
  claude preset ✓). No crontab exists yet.

---

## 4. Where we WANT to be (the target state)

1. **A stranger's repo goes from zero → governed in under 2 minutes** via
   either door (`npx skills add …whetstone` → skill → `wh init --claude`, or
   bare `wh` → wizard), with the wizard's payoff scan demonstrating value on
   *their* code. (b13 + v0.11.0)
2. **The owner's real repos live under governance for weeks**: hook feedback
   in-session, taste captured as it comes up, adherence trending, friction
   logged. (field-test playbook — `planning/field-test-playbook.md` — **still
   never executed**; interactive artifact exists)
3. **Janitor completes its exit test**: three consecutive clean nights on a
   real repo — correct cited PRs, zero spam, accurate digests. (7g6 → 766)
4. **The ratchet is real**: at least one external audit finding converted to a
   cited, eval-gated rule that never recurs. (ykf)
5. **Content grows only where usage demands** (4us resources wave, 6b4
   archetypes stay parked until adoption data exists).

**The honest headline gap: machinery is ~3 epics ahead of validation.** Every
capability is synthetically validated (independent subagents, adversarial
probes) but real-world hours are ~zero: no released wizard, no field-test
weeks, no Janitor nights. The next unit of effort should buy *evidence*, not
features.

---

## 5. The roadmap (ordered; do them roughly in this sequence)

| # | Action | Where | Why now |
|---|---|---|---|
| 1 | **Cut v0.11.0** (release protocol in CLAUDE.md: CHANGELOG → bump → tag → CI 4 binaries → install.sh verify → Homebrew formula) | whetstone | 15 commits / the whole wizard sit unreleased; b13 must point at a shipped binary |
| 2 | **`whetstone-b13`: publish the skill** on the npx skills channel; first-run bootstraps the binary | whetstone | Distribution is the only gap where shadcn/improve genuinely threatens us; SKILL.md is already agentskills.io format |
| 3 | **`janitor-7g6` hardening** (the 2 P0 majors first) | janitor (its own agent) | Both majors break the first real night; everything else waits on them |
| 4 | **`janitor-766`: the three nights** (real GitHub repo, planted violations, crontab/launchd) | janitor | The V1 definition of done; produces the first real-world evidence |
| 5 | **Run the field-test playbook** on 1–2 of the owner's real repos, 1–2 weeks, friction log → beads | whetstone | The adoption evidence everything else is gated on |
| 6 | **`whetstone-ykf`: the ratchet bridge** (+ README positioning) | whetstone | The strategic play vs improve; needs a real audit finding — pairs naturally with #5 |
| 7 | `whetstone-fed` + `janitor-2wh` leverage ranking; Homebrew tap publish; first pack re-validation cycle (`last_validated` bumps) | both | Polish + currency cadence, never yet exercised |
| 8 | Content epics `whetstone-4us` / `whetstone-6b4` | whetstone | Only if usage data says starter coverage is the bottleneck |

---

## 6. Invariants — do NOT change these without explicit owner sign-off

1. **High confidence or silence.** Every CLI rule: deterministic backing (ast
   `ast_query` / lint_proxy / formatter/tests/validators binding), doc
   citation, 3–5 goldens, `wh eval`-clean. If a linter already catches it,
   emit native config instead. Judgment-only taste → guidance store, never a
   signal-less rule.
2. **Boundary discipline** (`planning/skill-cli-boundary.md` — the contract):
   skill = judgment; CLI = deterministic oracles; TUI = skin over oracles for
   onboarding + review ONLY (everything else frozen); orchestration lives
   outside the `wh` binary (that's Janitor).
3. **Nothing enforceable without review** (human door reviews pre-import;
   agent door prints the review nudge). No TUI-only state; progress derived
   from artifacts; the single exception is `setup.dismissed` in whetstone.yaml,
   oracle-written.
4. **Janitor's six** (its AGENTS.md): deterministic queue only (no free-roam,
   ever) · agent never trusted (VERIFY decides) · never merge/force-push/touch
   human branches · ledger idempotency + pause/dismiss respected · hard
   budgets · fail open on infra, fail closed on verification.
5. **Gates before every push, both repos.** Whetstone: the 8. Janitor: fmt,
   clippy -D warnings, test.
6. **Releases** follow CLAUDE.md's protocol exactly (CHANGELOG + version bump
   before tag; never retag published releases).

---

## 7. Operational knowledge (saves the next agent an hour)

- **Recurring nuisance:** `tests/fixtures/whetstone/context/AGENTS.md`
  date-churns on every test run — `git checkout -- tests/fixtures/whetstone/context/AGENTS.md`
  before every commit.
- **PATH gotcha:** hooks and `.mcp.json` exec `wh`. If `wh --version` on PATH
  lags the repo, in-session enforcement/MCP silently no-op. Fix: `wh update`.
  (Currently 0.10.0 — fine until v0.11.0 ships, then update.)
- **Validation pattern (house style):** plan → build → **independent
  adversarial subagent validation** (empirical, per-bead acceptance) → fix →
  re-validate to APPROVE. Big plans get two validators (product + technical),
  iterated to agreement. Don't self-certify substantial epics.
- **Preview/read-only work:** anything touching pack resolution must use
  `SnapshotOptions { read_only, injected_packs }` — the default path WRITES
  `whetstone/.state` (and `.state` is gitignored, so `git status` proves
  nothing; hash the dir in tests).
- **Beads hygiene:** one bead in_progress at a time; close with `--reason`;
  file follow-ups instead of scope-creeping; `bd dolt push` if the Dolt remote
  is configured.
- **Key doc index:** product (`planning/product-spec.md`, `planning/mvp.md`) ·
  contract (`planning/skill-cli-boundary.md`) · onboarding design
  (`planning/tui-onboarding.md`) · field test (`planning/field-test-playbook.md`)
  · janitor design (`~/Development/janitor/PLAN.md`, as-built notes at bottom) ·
  schemas (`references/rule-schema.yaml`, `guidance-schema.yaml`,
  `conflicts-schema.md`) · corpus (`packs/README.md`).
- **Living artifacts** (claude.ai): the Whetstone field-test playbook
  (interactive checklist + friction log) and the Janitor pitch page — ask the
  owner for links if needed; the markdown sources of truth are in-repo.

---

## 8. One-paragraph summary for a hurried agent

Whetstone (standards: derive doc-cited rules once with judgment, enforce
forever deterministically — hook/CI/MCP/wizard) is feature-complete through
epic v5n but **unreleased past v0.10.0**; Janitor (overnight deterministic-queue
→ agent-fix → deterministic-verify PR harness) is implemented but has **two
night-breaking P0 bugs open** (`janitor-07i`, `janitor-3u5`) and has never run
a real night. The strategy question is settled (complement shadcn/improve via
the ratchet; win on determinism + persistence + distribution); the execution
question is evidence: release v0.11.0, publish the skill (`whetstone-b13`),
harden and run Janitor's three nights (`janitor-7g6` → `766`), run the
field-test playbook, then let real friction pick what gets built next. Check
`bd ready` in both repos before doing anything.
