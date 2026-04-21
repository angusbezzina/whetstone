# Whetstone Overview

> Last updated: 2026-04-21 | Version: 0.3.0 + `[Unreleased]` (Epic 3E — queued for 0.4.0)
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
│                                                                      │
│      Shortcut (personal preferences):                                │
│        wh rule add <id> --description ... --match 'regex'            │
│        → writes directly to .personal/rules/, status: approved       │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  3.  APPROVE                                      [Agent + User]     │
│      wh approve <rule-id>                                            │
│      wh approve --all [--dep X] [--confidence high]                  │
│      wh rule edit <id> --severity must      (bump as taste matures)  │
│      → status: candidate → approved                                  │
│      (Denial = delete the rule file. No separate deny command.)      │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  4.  GENERATE                                     [Binary]           │
│      wh actions        (chains: wh context + wh tests + wh lint)     │
│        --terse         (one-line-per-rule bootstrap, −50% tokens)    │
│      → whetstone/context/   AGENTS.md + AGENTS.<lang>.md sidecars    │
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
│      wh status   → rule_system_score + adherence_score + trend       │
│      wh report   → one-page markdown (PR-comment-friendly)           │
│      wh ci       → CI freshness gate                                 │
└────────────────────────┬────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────────────┐
│  7.  MAINTAIN                                     [Binary]           │
│      wh reinit        (re-resolve changed deps — version + content)  │
│      → .state/refresh-diff.json  (re_extraction_candidates + prompt) │
│      Loop back to step 2 when drift is detected.                     │
└─────────────────────────────────────────────────────────────────────┘

╔═══════════════════════════════════════════════════════════════════╗
║  JIT side-channel (any step):                                      ║
║    wh rules query --file <path> [--severity must] [--json]         ║
║  Agents should prefer this over re-reading AGENTS.md mid-turn.     ║
╚═══════════════════════════════════════════════════════════════════╝
```

The mermaid version — including the CI path and the agent's in-context work between calls — lives at [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd).

---

## What ships today (0.3.0 + `[Unreleased]`)

Everything marked `0.3.0` shipped in the lean refactor. Everything marked `3E` is merged to `main` in `[Unreleased]` and queued for 0.4.0.

| Module | Ship | Notes |
|--------|------|-------|
| Dependency detection | 0.3.0 | 3 languages · 4 manifest formats · monorepo-aware · incremental. `wh init --detect-only` for scan-only mode. |
| Source resolution | 0.3.0 | Parallel (rayon) · cached (7-day TTL) · resumable · crash-safe. |
| `wh init` bootstrap | 0.3.0 | Fast-first strategy · dependency ranking · readiness buckets · explicit handoff artifact. |
| State management | 0.3.0 | `extraction-handoff.json`, `refresh-diff.json`, `manifests.json`, `inventory.json`, `source-cache.json`, `metrics.jsonl`. Atomic writes. |
| Rule schema + validation | 0.3.0 | Two statuses (`candidate`, `approved`); mandatory deterministic signal; golden-example checks. |
| Extract + approve | 0.3.0 | `wh extract` prints the next worklist dep; `wh extract submit <bundle>` writes candidates (refuses id collisions); `wh approve` flips single or batch. |
| Agent context generation | 0.3.0 | 6 formats via `wh context` — agents.md, claude.md, .cursorrules, copilot, windsurf, codex. |
| Test generation | 0.3.0 | Tera templates → pytest / vitest / cargo test. `match` regex signals produce real checks; absent `match` becomes TODO stubs. |
| Lint overlay generation | 0.3.0 | `wh lint` emits `ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`. |
| `wh actions` chain | 0.3.0 | Runs context + tests + lint sequentially, fails fast on first error. |
| Deterministic enforcement | 0.3.0 | `wh check` — tree-sitter `ast_query` / `ast_scope` + regex + lint-proxy verification. |
| Rule review (read-only) | 0.3.0 | `wh review [--status]`, `wh review show <id>`, `wh review worklist`. |
| Status + CI gate | 0.3.0 | `wh status` (rule-system score) + `wh ci --fail-on stale`. |
| Refresh flow | 0.3.0 | `wh reinit` / `wh reinit --check`; diff at `.state/refresh-diff.json`. |
| Two-layer merge | 0.3.0 | Personal (gitignored) + project (committed). `wh init --personal` scaffolds the personal tree. |
| Custom sources | 0.3.0 | Arbitrary URLs declared in `whetstone.yaml`. |
| Triggers | 0.3.0 | `wh init --hooks` installs session + post-merge hooks; `wh init --ci --schedule=<cadence>` writes the CI workflow. |
| Binary distribution | 0.3.0 | GitHub Releases · `install.sh` · `wh` alias · `wh update` self-update. |
| Pre-push hook | 0.3.0 | `.githooks/pre-push` runs all 5 gates (clippy · cargo test · ruff · validate · pytest) before any push. |
| **JIT rule lookup** | 3E | `wh rules query --file <path>` returns the rules that apply to a file as JSON. Agents call it mid-turn instead of re-scanning AGENTS.md. |
| **Terse bootstrap + per-language sidecars** | 3E | `wh context --terse` / `wh actions --terse` shrinks AGENTS.md by ~51% on whetstone-self. Per-language `AGENTS.<lang>.md` sidecars (cross-linked) emit automatically when rules span >1 language. |
| **Personal-taste shortcuts** | 3E | `wh rule add <id> --description ... --match 'regex'` writes directly to `.personal/rules/` as approved. `wh rule edit` bumps severity/confidence (single or bulk via `--all --dep --category`). |
| **Custom source subscriptions** | post-3E | `wh source add/list/remove/fetch` for blogs, wikis, `llms.txt`, internal docs. Personal (gitignored) by default; `--project` for committed team subscriptions. Subscribed sources flow into the extraction worklist alongside detected deps. Underlying `sources.custom[]` config was 0.3.0; the CLI surface ships in `[Unreleased]`. |
| **Code-quality scoring** | 3E | `wh status` now returns BOTH `rule_system_score` (rule health) AND `adherence_score` (code quality, hybrid 60% clean-file + 40% severity-weighted). `.metrics.jsonl` captures violation counts per snapshot for trend. |
| **`wh report`** | 3E | One-page markdown: adherence + top 10 violations + drift + next actions. `--pr-comment` emits PR-friendly markdown with a `<!-- whetstone-report -->` tracking marker. |
| **Smarter reinit** | 3E | `refresh-diff.json` now carries `re_extraction_candidates` (per-rule, with current severity + source URL) and a canned `extraction_prompt`. Flags both version drift AND content-hash drift (docs rewritten without version bump). |
| **Format-validation tests** | 3E | Snapshot tests lock the minimum required markers in all 6 context formats so silent tool-parser divergence is caught pre-push. |
| **Measurement harness** | 3E | `scripts/measure-epic-3e.sh` + `planning/measurements/epic-3e-baseline.md` record token cost, runtime, and the epic acceptance deltas. |

---

## Where we're headed

### Recently shipped

#### Epic 3E: Active Whetstone — CLOSED 2026-04-20

> **Tracked as:** `whetstone-n34` | **Status:** Closed · 14/14 children + acceptance gate met

Identified four architectural gaps that kept Whetstone from answering "is my code in good shape?" as well as it answers "are my rules in good shape?". Landed as nine commits on `main` (currently `[Unreleased]` on track to ship in 0.4.0).

| Theme | Shipped |
|-------|---------|
| **A. Architecture — JIT consumption** | `wh rules query` · `wh context --terse` · per-language AGENTS sidecars |
| **B. Observability — project scoring** | `adherence_score` in `wh status` · violation-trend in `.metrics.jsonl` · `wh report` narrative |
| **C. Authoring — taste shortcuts** | `wh rule add` · `wh rule edit` (single + bulk) |
| **D. Maintenance — drift + hand-off** | `re_extraction_candidates` + canned `extraction_prompt` in `refresh-diff.json` · content-hash drift detection |
| **Measurement** | `scripts/measure-epic-3e.sh` + `planning/measurements/epic-3e-baseline.md` |

**Acceptance deltas achieved:**

- Session token cost: **−51.5%** on whetstone-self (target was ≥−40%).
- Time-to-add-personal-preference: **~10 s** via `wh rule add` (down from ~3–5 min).
- Repo-health in one command with a code-quality number: **`wh status.adherence_score`** + `wh report` narrative.
- `wh status` runtime: **15.7 ms** on whetstone-self (target ≤200 ms).

Dogfooding: whetstone-self was the target throughout. External-repo dogfood is a post-epic user task.

### Near-term

| Item | Status | Tracking |
|------|--------|----------|
| **`wh patterns` reinstatement** | Source commented-out at `src/detect_patterns.rs`; mod decl and CLI variant commented with `TODO(whetstone-aww)` markers. Reinstate when there's a clear use case. | `whetstone-e2r` (open) |
| **0.4.0 release** | Epic 3E is in `[Unreleased]`. Cut the tag, bump `Cargo.toml`, rebuild release binaries, update Homebrew. | TBD |
| **Config depth** | Only discovery + formats knobs are surfaced today. Extract timeouts, extraction settings, and resolve tuning live in code but aren't user-configurable. | TBD |
| **Archived planning cleanup** | `planning/archive/` docs still carry pre-0.3 command names. Not harmful; prune when touched. | TBD |

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
| `wh reinit` | Drift summary (version + content-hash); rewrites `refresh-diff.json` with `re_extraction_candidates` + canned `extraction_prompt`. `--check` exits non-zero on drift. |
| `wh set-sources` | Resolution results (lower-level slice of init). |
| `wh extract` | Top worklist dep + ranked sources + quota. `wh extract submit <bundle>` writes candidates. |
| `wh approve` | Flip candidates to approved. `<rule-id>` or `--all [--dep] [--confidence]`. |
| `wh rule add <id>` | Personal-taste shortcut. Writes a rule directly (default: `.personal/rules/`). `--match <regex>`, `--severity`, `--category`, `--lang`, `--dep`, `--project`. |
| `wh rule edit <id> \| --all` | Bumps `--severity` / `--confidence` in place; `--dry-run` to preview. Refuses candidate rules. |
| `wh source add/list/remove/fetch` | Subscribe to custom rule sources (blogs, wikis, `llms.txt`, internal docs). Default layer: personal (gitignored). `--project` for committed team subscriptions. Subscribed sources appear in the extraction worklist alongside detected deps. |
| `wh rules query` | JIT rule lookup. Filters: `--file <path>` (infers language), `--lang`, `--dep`, `--severity`, `--personal-only`, `--project-only`, `--full`. Preferred over re-scanning `AGENTS.md` mid-turn. |
| `wh context` | Agent context files under `whetstone/context/`. `--terse` for one-line-per-rule bootstrap; per-language `AGENTS.<lang>.md` sidecars emit automatically when rules span >1 language. |
| `wh tests` | Test scaffolds under `whetstone/evals/`. |
| `wh lint` | Linter overlays under `whetstone/lint/`. |
| `wh actions` | Chains context + tests + lint. Inherits `--terse` / `--lang` / `--personal`. |
| `wh check` | Deterministic rule scan (tree-sitter + regex + lint_proxy). |
| `wh validate` | Schema + fixture validation. |
| `wh status` | Both `rule_system_score` (rule health) AND `adherence_score` (code quality, hybrid formula). Violation counts + trend snapshot. |
| `wh report` | One-page markdown: adherence + top 10 violations + drift + next actions. `--pr-comment` for PR-friendly markdown. |
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
| `planning/measurements/epic-3e-baseline.md` | Token/runtime baselines + delta targets for Epic 3E |
| `planning/measurements/adherence-score-design.md` | Design-pass record for the hybrid adherence formula |
| `scripts/measure-epic-3e.sh` | Repeatable measurement harness |
| `.githooks/pre-push` | Pre-push gate — runs all 5 quality gates before any push |

---

*Whetstone sharpens the tools that write your code.*
