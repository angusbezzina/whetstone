# Whetstone Overview

> Last updated: 2026-04-20 | Version: 0.3.0
> Related reading: [`SKILL.md`](../SKILL.md) · [`references/workflow-matrix.md`](../references/workflow-matrix.md) · [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd)

---

## What it does

Whetstone is the **rule-intelligence layer** for your codebase. Point it at a project and it will:

1. **Read the manifests** (`pyproject.toml`, `package.json`, `Cargo.toml`, `requirements.txt`).
2. **Fetch the actual docs** of each dependency you use — `llms.txt`, registry READMEs, HTML docs, and changelogs from the last 18 months.
3. **Let an agent draft high-confidence coding rules** from those docs. You approve them.
4. **Generate committed outputs** — agent context (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`, …), linter overlays (ruff / biome / clippy), and runnable tests — that enforce those rules across your team.

It does **not** replace ruff / biome / clippy. It fills the gap between what those catch and what the docs say.

The **agent is the LLM.** There is no API key and no LLM client in the binary. Whetstone sits between your existing agent (Claude Code, Cursor, Codex, …) and the codebase, giving the agent deterministic JSON oracles to reason against.

---

## How it works

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

The mermaid version — including the CI path and the agent's in-context work between calls — lives at [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd).

---

## What ships today (0.3.0)

| Module | Status | Notes |
|--------|--------|-------|
| Dependency detection | Production-quality | 3 languages · 4 manifest formats · monorepo-aware · incremental. `wh init --detect-only` for scan-only mode. |
| Source resolution | Production-quality | Parallel (rayon) · cached (7-day TTL) · resumable · crash-safe. |
| `wh init` bootstrap | Production-quality | Fast-first strategy · dependency ranking · readiness buckets · explicit handoff artifact. |
| State management | Solid | `extraction-handoff.json`, `refresh-diff.json`, `manifests.json`, `inventory.json`, `source-cache.json`, `metrics.jsonl`. Atomic writes. |
| Rule schema + validation | Complete | Two statuses (`candidate`, `approved`); mandatory deterministic signal; golden-example checks. |
| Extract + approve | Implemented | `wh extract` prints the next worklist dep; `wh extract submit <bundle>` writes candidates (refuses id collisions); `wh approve` flips single or batch. |
| Agent context generation | Implemented | 6 formats via `wh context` — agents.md, claude.md, .cursorrules, copilot, windsurf, codex. |
| Test generation | Implemented | Tera templates → pytest / vitest / cargo test. `match` regex signals produce real checks; absent `match` becomes TODO stubs. |
| Lint overlay generation | Implemented | `wh lint` emits `ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`. |
| `wh actions` chain | Implemented | Runs context + tests + lint sequentially, fails fast on first error. |
| Deterministic enforcement | Implemented | `wh check` — tree-sitter `ast_query` / `ast_scope` + regex + lint-proxy verification. |
| Rule review (read-only) | Implemented | `wh review [--status]`, `wh review show <id>`, `wh review worklist`. |
| Status + CI gate | Implemented | `wh status` score 0–100 with drift detection; `wh ci --fail-on stale` for CI. |
| Refresh flow | Implemented | `wh reinit` / `wh reinit --check`; diff at `.state/refresh-diff.json`. |
| Two-layer merge | Implemented | Personal (gitignored) + project (committed). `wh init --personal` scaffolds the personal tree. |
| Custom sources | Implemented | Arbitrary URLs declared in `whetstone.yaml`. |
| Triggers | Implemented | `wh init --hooks` installs session + post-merge hooks; `wh init --ci --schedule=<cadence>` writes the CI workflow. |
| Binary distribution | Shipped | GitHub Releases · `install.sh` · `wh` alias · `wh update` self-update. |
| Pre-push hook | Shipped | `.githooks/pre-push` runs all 5 gates (clippy · cargo test · ruff · validate · pytest) before any push. |

---

## Where we're headed

### Near-term

#### Epic 3E: Active Whetstone

> **Tracked as:** `whetstone-n34` | **Status:** Open · 10 children

The goal-review on 2026-04-20 identified four gaps that keep Whetstone from answering "is my code in good shape?" as well as it answers "are my rules in good shape?". Epic 3E closes those gaps across four themes:

| Theme | Why | Representative children |
|-------|-----|--------------------------|
| **A. Token efficiency** | AGENTS.md is loaded every session regardless of what file is being edited; the ~236-line preamble is pure waste on repeat loads. | `whetstone-2gw` (per-language sidecars), `whetstone-ydw` (`--terse` + preamble trim) |
| **B. Real project scoring** | `wh status` measures rule-system health, not code quality. A repo with 1000 violations can still score 100. | `whetstone-0m0` (fold `wh check` in), `whetstone-90m` (top-line `adherence_score`), `whetstone-m2q` (violation trend in `.metrics.jsonl`) |
| **C. Less passive / query surface** | Agents can't ask "what rules apply to this file?"; personal preferences require hand-written YAML. | `whetstone-80x` (`wh rules query`), `whetstone-9uh` (`wh rule add` shortcut) |
| **D. Taste management** | `wh reinit` detects doc drift but doesn't suggest rule changes; bumping `should → must` means hand-editing YAML. | `whetstone-jrs` (auto-extract on reinit), `whetstone-5eb` (`wh rule edit` bulk mutations), `whetstone-awj` (changelog-driven severity suggestions) |

Epic closes when all ten children close — `bd show whetstone-n34` for the live dependency graph.

#### Other near-term items

| Item | Status | Tracking |
|------|--------|----------|
| **`wh patterns` reinstatement** | Source commented-out at `src/detect_patterns.rs`; mod decl and CLI variant commented with `TODO(whetstone-aww)` markers. Reinstate when there's a clear use case. | `whetstone-e2r` |
| **Format-validation tests** | `wh context` emits 6 formats (agents.md, .cursorrules, copilot, windsurf, codex) but zero tests verify they parse in the target tools. Silent divergence risk. | `whetstone-2r9` |
| **Config depth** | Only discovery + formats knobs are surfaced today. Extract timeouts, extraction settings, and resolve tuning live in code but aren't user-configurable. | TBD |
| **Deferred overview content cleanup** | Archived planning docs (epic retros, dogfood logs) still carry pre-0.3 command names in narrative form. Not harmful; will be pruned when touched. | TBD |

### Future concerns (not scoped to the lean core)

Explicitly out of scope for the near-term solo/local product. Revisit when teams start using Whetstone for reporting or compliance, or when the core loop proves stable enough to add breadth.

- **Technical-debt quantification** — aggregating violations into a time/effort estimate, showing debt trend over time, generating a one-page debt report for PRs. Valuable for team visibility and management reporting; not required for the solo workflow. The Epic 3E `adherence_score` + violation trend give most of the "is my code in good shape?" signal without needing hour estimates.
- **Local MCP server** exposing `wh rules query` / `wh check` as MCP tools for dynamic agent consumption during a turn. Dependent on `whetstone-80x` landing first.

### Longer-term (Epic 4: Platform + Registry)

- **Shared rule registry** — pre-extracted, community-ranked rules for popular deps.
- **Publishing** — users or teams publish rulesets (`extends: @user/fastapi-strict`).
- **Signal promotion (`wh evolve`)** — AI verdicts graduate to deterministic signals over time.
- **Whetstone as a Service** — GitHub App, pooled LLM access, Dependabot-model for coding conventions.

This is **TBD** rather than required for the local / single-user product. It becomes relevant once multi-user distribution and ecosystem effects matter.

### Deferred in the 0.3.0 lean refactor

Removed, not deprecated — code is gone (except patterns, which is on disk but commented out). Reintroduce only when the core loop proves stable.

| Removed feature | Was | Why deferred |
|-----------------|-----|--------------|
| `wh propose` / `wh apply` | Bundle diff + lifecycle transitions | Replaced by direct writes via `wh extract submit` + `wh approve`. |
| `wh promote` / `wh layers` | Move rules between 4 layers; inspect merge | Merge collapsed to personal + project. Promotion is `mv`. |
| `wh bench run` / `snapshot` | F1-scoring a rule corpus | Research tool; not part of the agent-coding-rules value loop. |
| `wh eval generate/run/calibrate` | AI-assisted judgment for ambiguous rules | Keeps heavy round-trips out of the hot path. Every rule now requires a deterministic signal. |
| `wh patterns` | Mine patterns from git / PRs / transcripts | **Preserved as commented-out code** — reinstatement tracked as `whetstone-e2r`. |
| `wh config show/validate` | Inspect effective config with provenance | Config still loads; the inspector is deferred. |
| Team `extends:` + built-in layer | Team ruleset distribution + `whetstone:recommended` | Reintroduce once multi-team demand is real. |
| AI eval signal strategy | `strategy: ai` signals + `ai_eval` config | Every rule now requires `ast`, `pattern`, or `lint_proxy`. |

---

## Design principles

1. **High confidence or silence.** Five trusted rules beat fifty noisy ones. Every rule needs a deterministic signal and a documentation citation. Under 90 % confident? Don't propose it.
2. **CLI as structured oracle.** The binary answers questions with JSON. The agent reasons between calls. The binary never orchestrates agent behaviour — `SKILL.md` teaches the workflow.
3. **The agent IS the LLM.** No API key in the binary. The user's existing agent performs extraction and judgment. Whetstone adds zero incremental cost.
4. **Complement, don't compete.** Whetstone fills the gap that ruff, biome, and clippy don't cover. It generates artifacts those tools consume.
5. **Generated outputs are the product.** A teammate who never installs Whetstone still gets every rule enforced (via generated tests in CI) and every agent guided (via committed context files). Whetstone is codegen, not a runtime dependency.
6. **Incremental by default.** Manifest fingerprinting, content hashing, cache TTL, resumable resolution. Don't redo work.
7. **Lean over comprehensive.** If a feature doesn't appear in the seven-command happy path, it belongs behind `--advanced` or gets deferred. Ceremony (lifecycle audit trails, proposal bundles, multi-layer merges) is dead weight at this stage.

---

## Commands

The complete canonical surface (no aliases):

| Command | What it returns |
|---------|----------------|
| `wh init` | Bootstrap: deps detected, sources resolved, extraction handoff written. `--detect-only` for scan only. `--personal` / `--hooks` / `--ci` for setup side-tasks. |
| `wh reinit` | Drift summary; rewrites `whetstone/.state/refresh-diff.json`. `--check` exits non-zero on drift. |
| `wh set-sources` | Resolution results (lower-level slice of init). |
| `wh extract` | Top worklist dep + ranked sources + quota. `wh extract submit <bundle>` writes candidates. |
| `wh approve` | Flip candidates to approved. `<rule-id>` or `--all [--dep] [--confidence]`. |
| `wh context` | Agent context files under `whetstone/context/`. |
| `wh tests` | Test scaffolds under `whetstone/evals/`. |
| `wh lint` | Linter overlays under `whetstone/lint/`. |
| `wh actions` | Chains context + tests + lint. |
| `wh check` | Deterministic rule scan (tree-sitter + regex + lint_proxy). |
| `wh validate` | Schema + fixture validation. |
| `wh status` | Health score, freshness, drift. |
| `wh ci` | Freshness gate with optional PR comment. |
| `wh review` | Rule inspection only (`[--status]`, `show <id>`, `worklist`). |
| `wh update` | Self-update the binary. |

All commands accept `--json` (auto when piped) and `--project-dir`. Full artifact I/O matrix in [`references/workflow-matrix.md`](../references/workflow-matrix.md).

---

## Supported languages

| Language | Manifest | Registry | Test output | Lint output |
|----------|---------|----------|-------------|-------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff |
| TypeScript | `package.json` | npm | vitest | biome |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy |

---

## Key files

| File | Purpose |
|------|---------|
| `SKILL.md` | The agent skill — workflow loaded by Claude Code, Cursor, etc. |
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
