# Whetstone Overview

> Last updated: 2026-04-20 | Version: 0.3.0 | Previous planning docs archived in `planning/archive/`
> See [`references/workflow-matrix.md`](../references/workflow-matrix.md) for the shipped command matrix.

---

## Deferred (0.3.0 lean refactor)

The 0.3.0 release collapses the surface to the seven-command happy path
(`wh init`, `wh extract`, `wh extract submit`, `wh approve`, `wh actions`,
`wh check`, `wh reinit`). The following features were removed and are
deferred until the core loop stabilizes: `wh promote` / `wh layers` (merge
collapsed to personal + project); `wh propose` / `wh apply` /
`wh review queue` / `wh review diff` (replaced by extract submit +
approve); `wh bench` / `wh eval` / `wh patterns` (benchmark corpus, AI
eval, pattern mining parked); `wh config show/validate` (config still
loads, inspector deferred); built-in rules and team `extends:` (only
project + personal layers remain).

---

## What Whetstone Is

Whetstone is the **rule-intelligence layer** for your codebase. It derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files — all from the same approved ruleset.

Other tools execute checks, review PRs, or apply fixes. Whetstone decides **which rules are worth enforcing in the first place**, proves them with source links and deterministic signals, and keeps them current as dependencies evolve.

---

## Architecture: CLI as Structured Oracle

Whetstone follows the 2026 consensus pattern for agentic developer tools: **the CLI answers questions with structured JSON, the agent reasons between calls, and the user has final say.**

```
Binary (deterministic)          Agent (judgment via SKILL.md)        User
──────────────────────          ────────────────────────────         ────
wh init --json             →    reads summary + content         →   confirms scope
                                applies extraction prompt
                                wh extract submit <bundle>      →   wh approve
wh validate --json         →    fixes errors if any
wh actions                 →    reports what was generated
wh check src/              →    self-verifies
wh status --json           →    reports health
```

Five CLI calls. The agent reasons between them. SKILL.md teaches it how. See `planning/whetstone-logic-flow.mmd` for the full visual.

### What the Binary Does

- Parse manifests (pyproject.toml, package.json, Cargo.toml, requirements.txt)
- Query registries (PyPI, npm, crates.io) and probe for llms.txt
- Cache resolution results with TTL, content hashing, crash-safe checkpointing
- Validate rule YAML against schema
- Generate test files, lint configs, and agent context files from approved rules (`wh context`, `wh tests`, `wh lint`, `wh actions`)
- Run deterministic enforcement via tree-sitter, regex, and lint-proxy validation (`wh check`)
- Write candidate rules from a bundle, refusing id collisions (`wh extract submit`)
- Flip candidate → approved by id or batch selectors (`wh approve`)
- Compute health scores, detect drift, gate CI pipelines (`wh status`, `wh ci`, `wh reinit`)
- All commands support `--json` for agent consumption

### What the Skill Does

- Read documentation content and decide what matters
- Apply the extraction prompt (high confidence or silence)
- Decompose rules into deterministic signals
- Draft a candidate bundle and submit it with `wh extract submit`
- Present candidates conversationally and guide the user through `wh approve`
- Compose CLI calls into a coherent workflow

### What the Binary Does NOT Do

- Make judgment calls about rule quality
- Orchestrate agent behaviour via state machines
- Require an API key or LLM client
- Auto-commit, auto-deploy, or touch source code outside `whetstone/`

---

## How It Works: The Seven-Command Loop

```
┌─────────────────────────────────────────────────────────────────────┐
│  1.  BOOTSTRAP                                    [Binary]           │
│      wh init                                                         │
│      → detect manifests  → resolve docs + changelogs                 │
│      → write .state/extraction-handoff.json                          │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  2.  EXTRACT                                      [Agent]            │
│      wh extract                       (prints top dep + sources)     │
│      ... agent reads docs, drafts a candidate bundle YAML ...        │
│      wh extract submit <bundle.yaml>                                 │
│      → rules/<lang>/<dep>.yaml  (status: candidate)                  │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  3.  APPROVE                                      [Agent + User]     │
│      wh approve <rule-id>                                            │
│      wh approve --all [--dep X] [--confidence high]                  │
│      → status: candidate → approved                                  │
│      (Denial = delete the rule file. No separate deny command.)      │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  4.  GENERATE                                     [Binary]           │
│      wh actions        (chains: wh context + wh tests + wh lint)     │
│      → whetstone/context/   AGENTS.md, CLAUDE.md, .cursorrules, …    │
│      → whetstone/evals/     pytest / vitest / cargo test scaffolds   │
│      → whetstone/lint/      ruff / biome / clippy overlays           │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  5.  VERIFY                                       [Binary]           │
│      wh check src/                                                   │
│      → tree-sitter AST queries + regex + lint_proxy verification     │
│      → exit 0 (clean) or exit 1 (violations)                         │
│      The agent's self-check before declaring a task done.            │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  6.  MONITOR                                      [Binary]           │
│      wh status        (health score 0–100, freshness, drift)         │
│      wh ci            (CI freshness gate with optional PR comment)   │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  7.  MAINTAIN                                     [Binary]           │
│      wh reinit        (re-resolve only changed deps)                 │
│      → .state/refresh-diff.json                                      │
│      Loop back to step 2 when drift is detected.                     │
└─────────────────────────────────────────────────────────────────────┘
```

The mermaid version of this flow — including the CI path and the agent's
in-context work between calls — lives at
[`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd).

---

## Current State (v0.3.0)

### What's Built and Working (0.3.0)

| Module | Status | Notes |
|--------|--------|-------|
| Dependency detection | Production-quality | 3 languages, 4 manifest formats, monorepo-aware, incremental. `wh init --detect-only` for scan-only mode. |
| Source resolution | Production-quality | Parallel (rayon), cached (7d TTL), resumable, crash-safe. |
| `wh init` orchestration | Production-quality | Fast-first strategy, dependency ranking, readiness buckets, explicit handoff artifact. |
| State management | Solid | `extraction-handoff.json`, `refresh-diff.json`, `manifests.json`, `inventory.json`, `source-cache.json`, `metrics.jsonl`. Atomic writes. |
| Rule schema + validation | Complete, trimmed | Two statuses (`candidate`, `approved`). Removed fields: `source_kind`, `linter_gap`, `risk`, `proposed_at`, `proposed_by`, `approved_at`, `denied_reason`, `deprecated_reason`, `superseded_by`. Golden-example checks intact. |
| Extract workflow | Implemented | `wh extract` prints the top worklist dep; `wh extract submit <bundle>` writes candidates directly and refuses id collisions. |
| Approve workflow | Implemented | `wh approve <id>`, `wh approve --all [--dep] [--confidence]`. Denial is `rm` on the rule file. |
| Agent context generation | Implemented | 6 formats (agents.md, claude.md, .cursorrules, copilot, windsurf, codex) via `wh context`. |
| Test generation | Implemented | Tera-backed templates; pytest / vitest / cargo test scaffolds via `wh tests`. `match` regexes produce real checks; missing-match signals become TODO stubs. |
| Lint overlay generation | Implemented | `wh lint` emits `ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`. Split out of `wh tests` in 0.3.0. |
| `wh actions` chain | Implemented | Runs context + tests + lint in sequence, fails fast on first error. |
| Deterministic enforcement | Implemented | `wh check` with tree-sitter `ast_query` / `ast_scope`, regex, lint-proxy verification. |
| Rule review (inspect only) | Implemented | `wh review [--status]`, `wh review show <id>`, `wh review worklist`. No lifecycle transitions — use `wh approve` / delete. |
| Status + CI gate | Implemented | `wh status` health scoring 0–100 with drift detection; `wh ci --fail-on stale` for CI. |
| Refresh flow | Implemented | `wh reinit` / `wh reinit --check`, reviewable diff at `.state/refresh-diff.json`. |
| Layers (simplified to 2) | Implemented | Personal (gitignored) + project (committed). `wh init --personal` scaffolds the personal tree. |
| Custom sources | Implemented | Arbitrary URLs declared in `whetstone.yaml` flow through the normal resolve pipeline. |
| Triggers | Implemented | `wh init --hooks` installs session + post-merge hooks; `wh init --ci --schedule=<cadence>` writes the CI workflow. |
| Binary distribution | Shipped | GitHub Releases, install.sh, `wh` alias, `wh update` self-update. |
| Pre-push hook | Shipped | `.githooks/pre-push` runs all 5 gates (clippy, cargo test, ruff, validate, pytest) before any push. Install via `git config core.hooksPath .githooks`. |

### Deferred in the 0.3.0 Lean Refactor

Not broken, not shipped — intentionally removed from the surface until the core loop stabilizes. Code is deleted (not stubbed), except pattern mining which is commented-out on disk.

| Removed Feature | Was | Why Deferred |
|-----------------|-----|--------------|
| `wh propose` / `wh apply` | Bundle diff + lifecycle transitions | Replaced by direct writes from `wh extract submit` + `wh approve`. |
| `wh promote` / `wh layers` | Move rules between 4 layers; inspect merge | Merge collapsed to personal + project. Promotion is `mv` between the two dirs. |
| `wh bench run` / `snapshot` | F1-scoring a rule corpus | Research tool; not part of the agent-coding-rules value loop. |
| `wh eval generate/run/calibrate` | AI-assisted judgment for ambiguous rules | Keeps heavy agent round-trips out of the hot path. Every rule now requires a deterministic signal. |
| `wh patterns` | Mine style patterns from git/PRs/transcripts | **Preserved as commented-out code** — user directive 2026-04-19 to reinstate later. |
| `wh config show/validate` | Inspect effective config with provenance | Config still loads; the inspector is deferred. |
| Team `extends:` + built-in layer | Team ruleset distribution + `whetstone:recommended` baseline | Reintroduce once multi-team demand is real. |
| AI eval signal strategy | `strategy: ai` on signals + `ai_eval` config block | Every rule now requires `ast`, `pattern`, or `lint_proxy`. |

### What's Still TBD

| Gap | Severity | Notes |
|-----|----------|-------|
| Config depth | Medium | Only discovery + formats knobs; no extraction settings, no resolve timeouts surfaced. |
| Shared registry / publishing | Deferred | Multi-user ruleset distribution — future work tied to Epic 4. |
| `wh evolve` | Deferred | Signal promotion from AI verdicts → deterministic signals. |
| Service / GitHub App layer | Deferred | Relevant once Whetstone expands beyond local / single-user use. |

### Dogfooding Results (2026-04-05)

First real rules extracted for Whetstone's own Rust deps (6 rules across serde_yaml, anyhow, reqwest, clap). Key findings:
- **Status pipeline works perfectly** — score 100, all dimensions correct
- **Context generation works** — Do/Don't sections, source URLs, good/bad examples
- **Test generation now writes real regex checks when rules include `match:`** — only signals without a concrete `match` stay as documented TODO scaffolds
- ~~**Content gap was the real blocker**~~ — **Fixed**: added 3-tier fallback (llms.txt → registry README → HTML conversion). All 10 deps now return content with medium confidence.
- **`extract --show` proved unnecessary** — doctor output includes content directly. Closed the bead.

---

## Delivered Epics

### Epic 1: Productise the Core Loop

> **Tracked as:** `whetstone-d2t` (superseded by follow-on epic `whetstone-nq8`) | **Status:** Closed / shipped

The core loop is now productised and shippable:

- `wh init` writes a durable extraction handoff under `whetstone/.state/extraction-handoff.json`
- `wh reinit` / `wh reinit --check` re-resolve changed sources and write `whetstone/.state/refresh-diff.json`
- README, SKILL, CLI help, and the workflow matrix now describe one coherent contract
- Dogfooding covered both Whetstone and an external mixed-language repo

This epic is complete; see Epic 3D below for the lean-refactor simplifications that followed.

---

### Epic 2: Layers + Triggers

> **Tracked as:** `whetstone-vkh` in beads | **Status:** Closed (2026-04-13); partly rolled back in Epic 3D

Originally scaled Whetstone from solo-developer extraction to a four-layer
policy system (`personal > project > team > built-in`). In the 0.3.0 lean
refactor the team and built-in layers were removed and `wh promote` / `wh layers`
were deleted. What survives from this epic:

- **Personal layer** — `whetstone/.personal/` is auto-gitignored by
  `wh init --personal`; `rules/`, `evals/`, `lint/`, and `context/` live
  under that root. Personal overrides project.
- **Trigger: session hook** — `wh init --hooks` installs
  `.claude/whetstone-session-hook.sh` and merges a `SessionStart` entry into
  `.claude/settings.json` without clobbering other hooks.
- **Trigger: post-merge hook** — Same command writes
  `.githooks/post-merge` and wires `core.hooksPath` when it is unset.
- **Trigger: scheduled CI** — `wh init --ci --schedule=<cadence>` writes
  `.github/workflows/whetstone-check.yml` running `wh status` + `wh ci`.

Removed: team `extends:`, built-in `whetstone:recommended`, `wh layers`,
`wh promote`, global `~/.whetstone/config.yaml`. See Epic 3D for rationale.

---

### Epic 3B: Structural Enforcement + Maintainable Generation

> **Tracked as:** `whetstone-52a` in beads | **Status:** Closed / shipped

- **Tree-sitter integration** — Python, TypeScript, and Rust parsers are wired into `wh check`
- **AST-backed enforcement** — `ast_query` and `ast_scope` signals now execute through tree-sitter instead of only falling back to regex
- **`wh check` command** — deterministic enforcement runner for source scans and lint-proxy config verification
- **Template-based codegen** — Tera templates now back generated context, tests, and lint overlay output
- **Built-in rule upgrades** — shipped rules now use tree-sitter-capable signals where appropriate

---

### Epic 3C: Policy Review Workflow + Trust Benchmarks

> **Tracked as:** `whetstone-gop` in beads | **Status:** Closed / shipped; mostly rolled back in Epic 3D

Originally shipped `wh review`, `wh apply` (approve/deny/deprecate/supersede), an audit trail, and a `wh bench` benchmark harness. In the lean refactor:

- **`wh review`** — kept as a read-only inspector (`show`, `worklist`, list by status).
- **`wh apply`** — removed. Replaced by `wh approve` (single / batch).
- **Audit trail (`review-log.jsonl`)** — removed. Git is the audit trail.
- **`wh bench`** — removed. Benchmark corpus deferred.
- **Refresh-driven review queue** — kept via `.state/refresh-diff.json` + `wh review worklist`.

---

### Epic 3D: Lean Refactor

> **Tracked as:** beads `whetstone-ff5`, `aww`, `7s3`, `5f4`, `beh`, `ldr`, `co7`, `987` | **Status:** Closed / shipped (0.3.0, 2026-04-20)

Collapsed the surface to the seven-command happy path after a design review concluded that ~40 % of the CLI was not pulling its weight toward the stated goal ("help agents write best-practice code aligned with the user's preferences, more efficiently than they could manage unaided").

- `wh doctor` merged into `wh init`; `wh refresh` renamed `wh reinit`; all nine command aliases removed.
- `wh propose` + `wh apply` replaced by `wh extract submit` + `wh approve`.
- `wh bench`, `wh eval`, `wh patterns` (commented), `wh promote`, `wh layers`, `wh config` pruned.
- Rule status lifecycle cut from five values (`candidate`, `approved`, `denied`, `deprecated`, `superseded`) to two (`candidate`, `approved`).
- `wh tests` no longer emits lint configs; split into `wh lint`.
- `wh actions` added to chain `wh context` + `wh tests` + `wh lint`.
- Layer merge collapsed from four layers to two (personal + project).
- Schema trimmed: removed `source_kind`, `linter_gap`, `risk`, `proposed_at`, `proposed_by`, `approved_at`, `denied_reason`, `deprecated_reason`, `superseded_by`.
- AI eval machinery (`.state/eval-*.json`, `strategy: ai`, `ai_eval` block) deleted.
- Pre-push hook bootstrap documented in `AGENTS.md` and `CLAUDE.md` after the hook mis-configuration allowed a failing push through.

Net: **–868 lines of docs, –40 % of CLI surface, all 100 Rust + 181 Python tests green, single-day landing window.**

---

## Remaining TBD Work

### Epic 4: Platform + Registry (TBD)

- **Shared rule registry** — Pre-extracted, community-ranked rules for popular deps
- **Publishing** — Users/teams publish rulesets (`extends: @user/fastapi-strict`)
- **Signal promotion** — AI verdicts → new deterministic signals over time
- **Rule evolution** — Violation tracking, prompt refinement (`wh evolve`)
- **Whetstone as a Service** — GitHub App, pooled LLM access, Dependabot-model for coding conventions

This work is intentionally **TBD** rather than required for the current local / single-user product. It matters more once Whetstone needs multi-user distribution, collaboration, and ecosystem/network effects.

---

## Design Principles

1. **High confidence or silence.** Five trusted rules beat fifty noisy ones. Every rule needs a deterministic signal and a documentation citation. If you're not 90%+ confident, don't propose it.

2. **CLI as structured oracle.** The binary answers questions with JSON. The agent reasons between calls. The binary never orchestrates agent behaviour. The SKILL.md teaches the workflow.

3. **The agent IS the LLM.** No API key, no LLM client in the binary. The user's existing agent (Claude Code, Cursor, Codex, etc.) performs extraction and judgment. Whetstone adds zero cost beyond what the user already pays.

4. **Complement, don't compete.** Whetstone fills gaps that ruff, biome, and clippy don't cover. It generates artifacts those tools consume. It doesn't replace them.

5. **Generated outputs are the product.** A teammate who never installs Whetstone still gets every rule enforced (via generated tests in CI) and every agent guided (via committed context files). Whetstone is a codegen tool, not a runtime dependency.

6. **Incremental by default.** Manifest fingerprinting, content hashing, cache TTL, resumable resolution. Don't redo work.

7. **Lean over comprehensive.** If a feature doesn't appear in the seven-command happy path, it belongs behind `--advanced` or gets deferred. Ceremony (lifecycle audit trails, proposal bundles, multi-layer merges, heavy CLI subcommand graphs) is dead weight at this stage. Cut anything that isn't pulling its weight.

---

## CLI Command Reference

The complete canonical surface (no aliases as of 0.3.0):

| Command | What It Returns |
|---------|----------------|
| `wh init` | Bootstrap: deps detected, sources resolved, extraction-handoff written. `--detect-only` for dep scan alone. `--personal` / `--hooks` / `--ci` for setup side-tasks. |
| `wh reinit` | Drift summary; rewrites `whetstone/.state/refresh-diff.json`; `--check` exits non-zero on drift (CI gate). |
| `wh set-sources` | Resolution results with cache stats. Lower-level slice of init; normally invoked implicitly. |
| `wh extract` | Top worklist dep + ranked sources + quota. `wh extract submit <bundle>` writes candidates and refuses id collisions. |
| `wh approve` | Flip candidate rules to approved. `<rule-id>` for single; `--all [--dep] [--confidence]` for batch. |
| `wh context` | Agent context files under `whetstone/context/` (AGENTS.md, CLAUDE.md, .cursorrules, copilot, windsurf, codex). |
| `wh tests` | Test scaffolds under `whetstone/evals/` (pytest / vitest / cargo test). |
| `wh lint` | Linter overlays under `whetstone/lint/` (ruff / biome / clippy). |
| `wh actions` | Chains context + tests + lint in one run. |
| `wh check` | Deterministic rule scan (tree-sitter + regex + lint_proxy). Exits non-zero on violations unless `--no-fail`. |
| `wh validate` | Schema + fixture validation pass/fail. CI-friendly. |
| `wh status` | Health score 0–100, freshness, coverage, drift. `--score`, `--history`, `--extraction-ready`. |
| `wh ci` | Freshness gate. `--fail-on stale` or `--fail-on needs_review` for PR gating. |
| `wh review` | Rule inspection only: list by status, `show <id>`, `worklist`. No lifecycle transitions — use `wh approve` or delete the rule file. |
| `wh update` | Self-update the binary from GitHub Releases. Does NOT touch rules. |

All commands support `--json` (auto-enabled when piped), and project-scoped commands support `--project-dir`. For the full matrix — including which lifecycle step each command serves and which artifacts it reads/writes — see [`references/workflow-matrix.md`](../references/workflow-matrix.md).

---

## Supported Languages

| Language | Manifest | Registry | Test Output | Lint Output |
|----------|---------|----------|-------------|-------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff |
| TypeScript | `package.json` | npm | vitest | biome |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy |

---

## Beads State

Completed major epics:

- `whetstone-nq8` — core-loop contract, handoffs, built-ins, dogfooding
- `whetstone-vkh` — layers + triggers (partly rolled back by 3D)
- `whetstone-52a` — structural enforcement + maintainable generation
- `whetstone-gop` — review/apply workflow + trust benchmarks (mostly rolled back by 3D)
- `whetstone-1n3` — extraction UX and config depth (superseded by the lean refactor)
- `whetstone-{ff5, aww, 7s3, 5f4, beh, ldr, co7, 987}` — Epic 3D lean refactor (0.3.0)

Still TBD:

- `whetstone-s2a` — platform + registry
- `whetstone-64x` — planning/whetstone-overview.md rewrite (closed by this doc update)

Run `bd ready` to see any newly-opened follow-up work. Use `bd memories` to inspect durable design directives (the lean-over-comprehensive rule, the patterns-commented-out directive, the pre-push preflight).

---

## Key Files

| File | Purpose |
|------|---------|
| `SKILL.md` | Agent skill — the extraction/approval workflow the agent loads |
| `AGENTS.md` | Universal agent instructions for this repo |
| `CLAUDE.md` | Claude Code-specific instructions |
| `CHANGELOG.md` | Release notes, including the 0.3.0 migration table |
| `references/extraction-prompt.md` | The extraction prompt — core IP |
| `references/rule-schema.yaml` | Rule YAML format specification |
| `references/signal-strategies.md` | Signal decomposition guide |
| `references/workflow-matrix.md` | Shipped command matrix with artifact I/O |
| `references/handoff-schema.md` | `.state/*.json` contracts |
| `planning/whetstone-logic-flow.mmd` | Visual flow chart (mermaid) |
| `.githooks/pre-push` | Pre-push gate — runs all 5 quality gates before any push |

---

*Whetstone sharpens the tools that write your code.*
