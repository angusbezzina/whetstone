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
- Generate test files, lint configs, and agent context files from approved rules
- Run deterministic enforcement via tree-sitter, regex, and lint-proxy validation (`wh check`)
- Apply lifecycle transitions and persist audit logs (`wh review` / `wh apply`)
- Run benchmark corpora and regression gates (`wh bench`)
- Compute health scores, detect drift, gate CI pipelines
- All commands support `--json` for agent consumption

### What the Skill Does

- Read documentation content and decide what matters
- Apply the extraction prompt (high confidence or silence)
- Decompose rules into deterministic signals
- Present candidates conversationally and handle approval
- Propose candidate YAML and guide the user through review/apply decisions
- Compose CLI calls into a coherent workflow

### What the Binary Does NOT Do

- Make judgment calls about rule quality
- Orchestrate agent behaviour via state machines
- Require an API key or LLM client
- Auto-commit, auto-deploy, or touch source code outside `whetstone/`

---

## Current State (v0.2.0)

### What's Built and Working

| Module | Status | Notes |
|--------|--------|-------|
| Dependency detection | Production-quality | 3 languages, 4 manifest formats, monorepo-aware, incremental |
| Source resolution | Production-quality | Parallel (rayon), cached (7d TTL), resumable, crash-safe |
| Doctor orchestration | Strong | Fast-first strategy, dependency ranking, readiness buckets, explicit handoff artifact |
| State management | Solid | 4 stores plus refresh-diff + extraction-handoff, atomic writes, lifecycle tracking |
| Rule schema + validation | Complete | Full enum validation, golden example checks, lifecycle status transitions |
| Agent context generation | Implemented | 6 formats (agents.md, claude.md, .cursorrules, copilot, windsurf, codex) |
| Test + lint generation | Implemented | Tera-backed templates, pytest/vitest/cargo test outputs, ruff/biome/clippy overlays |
| Deterministic enforcement | Implemented | `wh check` with tree-sitter `ast_query`, `ast_scope`, regex, and lint-proxy verification |
| Review / apply workflow | Implemented | `wh review`, `wh apply`, lifecycle transitions, batch apply, audit log |
| Benchmark harness | Implemented | `wh bench run` / `snapshot`, corpus under `benchmarks/`, CI-friendly regression gating |
| Status + CI gate | Implemented | Health scoring 0-100, configurable thresholds, PR comments |
| Refresh flow | Implemented | `wh reinit` / `wh reinit --check`, reviewable diff under `whetstone/.state/refresh-diff.json` |
| AI eval runner | Implemented | threshold gating, eval generate/run/calibrate, file-based agent handoff |
| Built-in rules | Implemented | `whetstone:recommended` baseline for Rust, Python, and TypeScript |
| Custom sources | Implemented | Arbitrary URLs declared in `whetstone.yaml` flow through the normal resolve pipeline |
| Layers + triggers | Implemented | personal/project/team/built-in layering, promote, hooks, scheduled CI, global personal config |
| Pattern mining | Implemented | Transcripts, git history, PR comments |
| Binary distribution | Shipped | GitHub Releases, install.sh, `wh` alias, `wh update` self-update |

### What's Still TBD

| Gap | Severity | Notes |
|-----|----------|-------|
| Config depth | Medium | Only discovery + formats; no extraction settings, no timeouts |
| Shared registry / publishing | Deferred / TBD | `@user/config` is parseable in `extends:` but reports `not_implemented` — multi-user distribution is future work |
| `wh evolve` | Deferred | Signal promotion from AI verdicts → deterministic signals is not yet implemented |
| Service / GitHub App layer | Deferred / TBD | Could come later if Whetstone expands beyond primarily local / single-user use |

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
- `wh eval generate|run|calibrate` provide explicit file-based handoffs for AI judgment and calibration
- README, SKILL, CLI help, and the workflow matrix now describe one coherent contract
- Built-in rules exist for Rust, Python, and TypeScript
- Dogfooding covered both Whetstone and an external mixed-language repo

- This epic is complete; the next shipped capabilities now live in Epics 3B and 3C below.

---

### Epic 2: Layers + Triggers

> **Tracked as:** `whetstone-vkh` in beads | **Status:** Closed (2026-04-13)

Scaled Whetstone from solo-developer extraction to a preference-aware policy
system. Every child below ships with integration-test coverage.

- **Personal layer** — `whetstone/.personal/` is auto-gitignored by
  `wh init --personal`; `rules/`, `evals/`, `lint/`, and `context/` all live
  under that root.
- **Team layer** — `whetstone.yaml extends:` accepts `whetstone:recommended`,
  `github.com/owner/repo` (git-cloned into `whetstone/.cache/teams/`), plus
  `@user/config` / `https://…` forms that are parsed and report
  `not_implemented` until the registry lands.
- **Layer merge** — `LayerSet::merge()` implements
  `personal > project > team > built-in` with per-layer deny lists. `wh layers`
  surfaces the merged set for debugging.
- **Promote command** — `wh promote <rule-id> --to personal|project|team`
  moves (or with `--keep-source`, copies) rule files between layers.
  Monotonic: downward promotions are rejected.
- **Trigger: session hook** — `wh init --hooks` installs
  `.claude/whetstone-session-hook.sh` and merges a `SessionStart` entry into
  `.claude/settings.json` without clobbering other hooks.
- **Trigger: post-merge hook** — Same command writes
  `.githooks/post-merge` and wires `core.hooksPath` when it is unset.
- **Trigger: scheduled CI** — `wh init --ci --schedule=<cadence>` writes
  `.github/workflows/whetstone-check.yml` running `wh status` + `wh ci`.
- **Global personal config** — `~/.whetstone/config.yaml` merges into every
  project's resolved `WhetstoneConfig`; deny lists union, explicit fields in
  the project override.

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

> **Tracked as:** `whetstone-gop` in beads | **Status:** Closed / shipped

- **`wh review`** — list rules by lifecycle status, inspect per-rule context, build a refresh review queue
- **`wh apply`** — approve / deny / deprecate / supersede rules without hand-editing YAML
- **Audit trail** — every lifecycle transition appends to `whetstone/.state/review-log.jsonl`
- **Refresh-driven review** — `extraction-handoff.json` + `refresh-diff.json` now support focused review queues for changed policy
- **Benchmark harness** — `wh bench run` / `snapshot` replay corpora and report precision / recall / F1 across deterministic, layered, and eval scenarios

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

---

## CLI Command Reference

The complete canonical surface (no aliases as of 0.3.0):

| Command | What It Returns |
|---------|----------------|
| `wh init` | Bootstrap: deps detected, sources resolved, extraction-handoff written. `--detect-only` for dep scan alone |
| `wh reinit` | Drift summary; rewrites `whetstone/.state/refresh-diff.json`; `--check` gates CI |
| `wh set-sources` | Resolution results with cache stats (advanced; normally invoked via `wh init`) |
| `wh extract` | Top worklist dep + ranked sources; `wh extract submit <bundle>` writes candidates |
| `wh approve` | Flip candidate rules to approved (single or batch via `--all --dep --confidence`) |
| `wh context` | Agent context files under `whetstone/context/` |
| `wh tests` | Test scaffolds under `whetstone/evals/` |
| `wh lint` | Linter overlays under `whetstone/lint/` (ruff / biome / clippy) |
| `wh actions` | Chains context + tests + lint |
| `wh check` | Deterministic rule scan (tree-sitter + regex + lint-proxy) |
| `wh validate` | Schema validation pass/fail |
| `wh status` | Health score, freshness, coverage, drift |
| `wh ci` | Pass/fail freshness check with optional PR comment |
| `wh review` | List rules by status; `wh review show <id>`, `wh review worklist` |
| `wh update` | Self-update the binary |
| `wh review` | — | Rule lifecycle listing, per-rule inspection, refresh review queue |
| `wh apply` | — | Lifecycle transitions (approve / deny / deprecate / supersede) |
| `wh bench` | — | Benchmark corpus run / snapshot with regression gating |
| `wh eval` | — | `generate`/`run`/`calibrate` — AI eval lifecycle with file-based agent handoff |
| `wh patterns` | `detect-patterns` | Discovered style patterns from transcripts/git/PRs |
| `wh update` | — | Self-update the `whetstone` binary from GitHub Releases (does NOT touch rules) |

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
- `whetstone-vkh` — layers + triggers
- `whetstone-52a` — structural enforcement + maintainable generation
- `whetstone-gop` — review/apply workflow + trust benchmarks

Still TBD:

- `whetstone-s2a` — platform + registry

Run `bd ready` to see any newly-opened follow-up work.

---

## Key Files

| File | Purpose |
|------|---------|
| `SKILL.md` | Agent skill — the extraction/approval product *(rewrite in Epic 1)* |
| `AGENTS.md` | Universal agent instructions for this repo |
| `CLAUDE.md` | Claude Code-specific instructions |
| `references/extraction-prompt.md` | The extraction prompt — core IP |
| `references/rule-schema.yaml` | Rule YAML format specification |
| `references/signal-strategies.md` | Signal decomposition guide |
| `planning/whetstone-logic-flow.mmd` | Visual flow chart (mermaid) |
| `planning/archive/layer-system.md` | Layer system + trigger modes design (Epic 2) |
| `planning/archive/product-spec.md` | Full product vision (reference) |

---

*Whetstone sharpens the tools that write your code.*
