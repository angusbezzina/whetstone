# Whetstone Roadmap v2

> Last updated: 2026-04-13 | Version: 0.2.0 | Previous planning docs archived in `planning/archive/`
> See [`references/workflow-matrix.md`](../references/workflow-matrix.md) for the shipped command matrix.

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
wh doctor --json           →    reads summary + content         →   confirms scope
                                applies extraction prompt
                                presents candidate rules        →   approve / deny / edit
                                writes YAML to rules/
wh validate --json         →    fixes errors if any
wh context + wh tests      →    reports what was generated
wh status --json           →    reports health
```

Five CLI calls. The agent reasons between them. SKILL.md teaches it how. See `planning/whetstone-logic-flow.mmd` for the full visual.

### What the Binary Does

- Parse manifests (pyproject.toml, package.json, Cargo.toml, requirements.txt)
- Query registries (PyPI, npm, crates.io) and probe for llms.txt
- Cache resolution results with TTL, content hashing, crash-safe checkpointing
- Validate rule YAML against schema
- Generate test files, lint configs, and agent context files from approved rules
- Compute health scores, detect drift, gate CI pipelines
- All commands support `--json` for agent consumption

### What the Skill Does

- Read documentation content and decide what matters
- Apply the extraction prompt (high confidence or silence)
- Decompose rules into deterministic signals
- Present candidates conversationally and handle approval
- Write approved rule YAML directly to the repo
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
| Test + lint generation | Implemented | pytest, vitest, cargo test with real regex checks + ruff, biome, clippy overlays |
| Status + CI gate | Implemented | Health scoring 0-100, configurable thresholds, PR comments |
| Refresh flow | Implemented | `wh refresh` / `wh refresh --check`, reviewable diff under `whetstone/.state/refresh-diff.json` |
| AI eval runner | Implemented | threshold gating, eval generate/run/calibrate, file-based agent handoff |
| Built-in rules | Implemented | `whetstone:recommended` baseline for Rust, Python, and TypeScript |
| Custom sources | Implemented | Arbitrary URLs declared in `whetstone.yaml` flow through the normal resolve pipeline |
| Pattern mining | Implemented | Transcripts, git history, PR comments |
| Binary distribution | Shipped | GitHub Releases, install.sh, `wh` alias, `wh update` self-update |

### What's Missing

| Gap | Severity | Notes |
|-----|----------|-------|
| Config depth | Medium | Only discovery + formats; no extraction settings, no timeouts |
| Tera templates | Medium | Codegen uses string concatenation; works but doesn't scale |
| tree-sitter AST signals | Medium | `ast` signals fall back to regex; real tree-sitter analysis is not yet wired up |
| Layer system | Deferred | No personal/team layers, no promote, no extends |
| Trigger modes | Deferred | No session/post-merge/scheduled hooks |
| Shared registry | Deferred | No community-ranked published rulesets |

### Dogfooding Results (2026-04-05)

First real rules extracted for Whetstone's own Rust deps (6 rules across serde_yaml, anyhow, reqwest, clap). Key findings:
- **Status pipeline works perfectly** — score 100, all dimensions correct
- **Context generation works** — Do/Don't sections, source URLs, good/bad examples
- **Test generation produces TODO scaffolds** — compiles but doesn't check anything without tree-sitter
- ~~**Content gap was the real blocker**~~ — **Fixed**: added 3-tier fallback (llms.txt → registry README → HTML conversion). All 10 deps now return content with medium confidence.
- **`extract --show` proved unnecessary** — doctor output includes content directly. Closed the bead.

---

## The Plan

Three epics, sequenced. Each builds on the last.

### Epic 1: Productise the Core Loop

> **Tracked as:** `whetstone-d2t` in beads | **Status:** Open, ready to start

Close the gap between Whetstone's strong infrastructure and its missing product surface. By completion, a user can run the full workflow end-to-end with real rules.

**Phase 1 — Done:**

| Bead | Work | Status |
|------|------|--------|
| ~~`whetstone-exd`~~ | Dogfood: extract rules for Whetstone's Rust deps | **Closed** — 6 rules across 4 deps |
| ~~`whetstone-dwm`~~ | Validate generate-context on real rules | **Closed** — works correctly |
| ~~`whetstone-6bm`~~ | Validate generate-tests on real rules | **Closed** — scaffolds compile, tree-sitter needed for real checks |
| ~~`whetstone-4aq`~~ | extract --show/--list commands | **Closed** — proved unnecessary by dogfooding |

**Phase 2 — Current focus (ordered by impact):**

| Priority | Bead | Work | Depends On |
|----------|------|------|-----------|
| Do first | `whetstone-la2` | Rewrite SKILL.md extraction workflow | — |
| Do first | `whetstone-dg4` | Custom source support (arbitrary URLs) | — |
| Do first | `whetstone-kp3` | Built-in rule system | — |
| Then | `whetstone-e51` | Curate built-in rules (3 languages) | kp3 |
| Then | `whetstone-t32` | Diff-based update command | — |
| Then | `whetstone-1dp` | Expand whetstone.yaml config depth | dg4 |
| Defer | `whetstone-d41` | Integrate tera template engine | — |
| Defer | `whetstone-5wg` | Harden doctor/status UX from dogfooding | — |

**Phase 3 — Eval + AST:**

| Bead | Work | Depends On |
|------|------|-----------|
| `whetstone-esp` | tree-sitter integration (Python, TS, Rust) | — |
| `whetstone-16c` | tree-sitter signal analysis | esp |
| `whetstone-7zr` | AI eval definition generation | — |
| `whetstone-71o` | Threshold gating logic | 7zr |
| `whetstone-ka5` | Agent-mediated eval runner + calibration | 71o, esp |

**Ready to start now (no blockers):** `la2`, `d41`, `dg4`, `5wg`, `t32`, `kp3`, `esp`, `7zr`

**Not in this epic:** Layer system, trigger modes, promote command, shared registry.

---

### Epic 2: Layers + Triggers

> **Planned in:** `planning/archive/layer-system.md` | **Status:** Not started

Scale Whetstone from solo developer to org-wide policy.

- **Personal layer** — `whetstone/.personal/` (gitignored, local only)
- **Team layer** — Shared config via `extends:` from external repos
- **Layer merge** — personal > project > team > built-in cascade
- **Promote command** — Move rules between layers
- **Trigger: session hook** — `wh status` on session start
- **Trigger: post-merge hook** — Drift check after `git pull`
- **Trigger: scheduled CI** — Periodic freshness checks
- **Global personal config** — `~/.whetstone/config.yaml`

Depends on Epic 1 completion (needs working extraction flow, built-in rules, validated pipeline).

---

### Epic 3: Platform (Future)

- **Shared rule registry** — Pre-extracted, community-ranked rules for popular deps
- **Publishing** — Users/teams publish rulesets (`extends: @user/fastapi-strict`)
- **Signal promotion** — AI verdicts → new deterministic signals over time
- **Rule evolution** — Violation tracking, prompt refinement (`wh evolve`)
- **Whetstone as a Service** — GitHub App, pooled LLM access, Dependabot-model for coding conventions

Not planned in detail. Depends on community adoption and Epic 1-2 validation.

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

| Command | Aliases | What It Returns |
|---------|---------|----------------|
| `wh doctor` | `start` | Summary: deps found, sources resolved, readiness, recommendations, extraction-handoff artifact |
| `wh refresh` | `refresh-rules` | Drift summary; rewrites `whetstone/.state/refresh-diff.json`; `--check` gates CI |
| `wh init` | `deps`, `detect-deps` | Detected dependencies with counts and drift |
| `wh set-sources` | `sources`, `resolve-sources` | Resolution results with cache stats |
| `wh validate` | `validate-rules` | Schema validation pass/fail |
| `wh context` | `generate-context` | Generated agent context files |
| `wh tests` | `generate-tests` | Generated test files + lint configs |
| `wh status` | — | Health score, freshness, coverage, drift |
| `wh ci` | `check`, `ci-check` | Pass/fail with optional PR comment |
| `wh eval` | — | `generate`/`run`/`calibrate` — AI eval lifecycle with file-based agent handoff |
| `wh patterns` | `detect-patterns` | Discovered style patterns from transcripts/git/PRs |
| `wh update` | — | Self-update the `whetstone` binary from GitHub Releases (does NOT touch rules) |

All commands support `--json` (auto-enabled when piped) and `--project-dir`. For the full matrix — including which lifecycle step each command serves and which artifacts it reads/writes — see [`references/workflow-matrix.md`](../references/workflow-matrix.md).

---

## Supported Languages

| Language | Manifest | Registry | Test Output | Lint Output |
|----------|---------|----------|-------------|-------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff |
| TypeScript | `package.json` | npm | vitest | biome |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy |

---

## Beads State

Active work is tracked under epic `whetstone-nq8` (reconcile contract, explicit handoffs,
broaden baseline coverage, dogfood). Run `bd ready` for the next available child.

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
