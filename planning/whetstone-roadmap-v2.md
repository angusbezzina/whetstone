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
| Shared registry | Deferred | `@user/config` is parseable in `extends:` but reports `not_implemented` — a community-ranked registry is still future work |
| `wh evolve` | Deferred | Signal promotion from AI verdicts → deterministic signals is not yet implemented |

### Dogfooding Results (2026-04-05)

First real rules extracted for Whetstone's own Rust deps (6 rules across serde_yaml, anyhow, reqwest, clap). Key findings:
- **Status pipeline works perfectly** — score 100, all dimensions correct
- **Context generation works** — Do/Don't sections, source URLs, good/bad examples
- **Test generation now writes real regex checks when rules include `match:`** — only signals without a concrete `match` stay as documented TODO scaffolds
- ~~**Content gap was the real blocker**~~ — **Fixed**: added 3-tier fallback (llms.txt → registry README → HTML conversion). All 10 deps now return content with medium confidence.
- **`extract --show` proved unnecessary** — doctor output includes content directly. Closed the bead.

---

## The Plan

Three epics, sequenced. Each builds on the last.

### Epic 1: Productise the Core Loop

> **Tracked as:** `whetstone-d2t` (superseded by follow-on epic `whetstone-nq8`) | **Status:** Closed / shipped

The core loop is now productised and shippable:

- `wh doctor` writes a durable extraction handoff under `whetstone/.state/extraction-handoff.json`
- `wh refresh` / `wh refresh --check` re-resolve changed sources and write `whetstone/.state/refresh-diff.json`
- `wh eval generate|run|calibrate` provide explicit file-based handoffs for AI judgment and calibration
- README, SKILL, CLI help, and the workflow matrix now describe one coherent contract
- Built-in rules exist for Rust, Python, and TypeScript
- Dogfooding covered both Whetstone and an external mixed-language repo

Remaining core-loop improvements are intentionally separated into later epics:

- **Deferred to Epic 3B:** tree-sitter-backed structural checks and template-engine cleanup
- **Deferred to Epic 4:** registry / evolution / service work

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

The first two epics are complete. Remaining work is intentionally deferred:

- `whetstone-52a` — Epic 3B: structural enforcement and maintainable generation
- `whetstone-s2a` — Epic 4: platform + registry

Run `bd ready` to see the next non-deferred follow-up when those epics are resumed.

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
