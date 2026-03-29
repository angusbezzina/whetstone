# Whetstone

> Sharpen the tools that write your code.

Whetstone is a **rule-intelligence layer** that derives coding rules from the documentation of your actual dependencies. It decomposes rules into deterministic checks and generates native tests, lint configs, and agent context files — all from the same approved ruleset.

Other tools execute checks, review pull requests, or apply fixes. Whetstone decides **which rules are worth enforcing** in the first place, why they matter, and how they map to deterministic enforcement and agent guidance.

It's a codegen tool, not a runtime dependency. A teammate who never installs Whetstone still gets every rule enforced through standard CI and every agent guided by current instructions.

## Why Whetstone?

**Rules go stale.** Linter configs and coding conventions are written once at project setup. Dependencies ship new versions, deprecate APIs, and introduce better patterns. Nobody updates the rules. Agents keep writing code against outdated practices.

**Dependency-specific best practices are unenforced.** Standard linters catch syntax and formatting. They don't know that FastAPI docs recommend `async def` for route handlers, or that Pydantic deprecated `.schema()` in favor of `model_json_schema()`. These are the rules that matter most — and nothing catches them.

**Agents aren't told what they need to know.** `AGENTS.md` and `.cursorrules` are written once by hand — if they're written at all — and never updated when dependencies evolve.

Whetstone solves all three. It treats documentation as a living source of truth, converts it into enforceable checks, and keeps everything current as your dependencies evolve.

### High confidence or silence

5 rules you trust completely beats 50 you have to review. Whetstone only proposes rules backed by specific documentation with deterministic signals. A project with 40 dependencies might get rules for 8 of them — those are the 8 that have something worth enforcing.

### What Whetstone is NOT

Whetstone is not a general AI code reviewer, a replacement for ruff/biome/clippy, or a broad semantic-eval platform. It complements existing tools by filling the gap between what linters catch and what dependency docs recommend.

## Quick Start

**Prerequisites:** Rust toolchain (for building from source), or download a release binary. Git and internet access for registry lookups.

### Install

```bash
# Build from source
cargo install --path .

# Or use directly from the repo
cargo build --release
./target/release/whetstone --help
```

### Recommended repo setup for contributors

Enable the repo-managed pre-push hook so local pushes run the same quality gates used in CI:

```bash
git config core.hooksPath .githooks
chmod +x .githooks/pre-push
```

### Usage

```bash
# 1. Run the doctor — one command from zero to working rules
whetstone doctor
# → detects dependencies from pyproject.toml / package.json / Cargo.toml
# → resolves documentation URLs from registries, probes for llms.txt
# → outputs extraction context for the agent

# 2. The agent reads the doctor output, proposes rules, you approve each one

# 3. Generate tests and agent context from approved rules
whetstone generate-context
whetstone generate-tests
# → pytest/vitest/cargo test files in whetstone/evals/
# → lint overlays in whetstone/lint/
# → agent context files in whetstone/context/

# 4. Check project health anytime
whetstone status
```

> **Agent skill mode:** When using Whetstone as an agent skill, say "whetstone doctor" or "whetstone status" and the agent runs the corresponding command. The binary handles everything — no Python runtime required.

## Canonical Workflow

Whetstone follows a six-step lifecycle. The doctor command handles steps 1-2 automatically.

| Step | Command | What happens |
|------|---------|-------------|
| **1. Detect** | `doctor` (or `detect-deps`) | Scan manifests for dependencies |
| **2. Resolve** | `doctor` (or `resolve-sources`) | Resolve docs URLs from registries, probe for llms.txt |
| **3. Extract** | Agent-mediated | LLM reads docs, proposes candidate rules |
| **4. Approve** | Agent-mediated | User reviews each rule (approve/edit/deny/skip) |
| **5. Generate** | `generate-tests` + `generate-context` | Produce tests, lint configs, agent context |
| **6. Monitor** | `status` / `ci-check` | Track freshness, drift, and health |

When dependencies update, run `detect-deps --changed-only` to see what drifted, then re-extract only the changed sources.

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│  Rust binary (deterministic)       Agent (LLM-mediated)     │
│                                                             │
│  detect-deps ─────────┐                                     │
│  resolve-sources ─────┤── doctor ──→  Extract rules         │
│                       │       │       (agent reads docs,    │
│                       │       │        proposes rules)      │
│                       │       │             ↓               │
│                       │       │       Approve rules         │
│                       │       │       (user reviews         │
│                       │       │        each one)            │
│                       │       │             ↓               │
│  generate-tests ──────┤───────┤── writes approved YAML      │
│  generate-context ────┘       │                             │
│                               │                             │
│  status ── health score, drift detection, next actions      │
│  ci-check ── CI gating, PR comments                        │
└─────────────────────────────────────────────────────────────┘
```

**The binary handles deterministic work:** dependency detection, URL resolution, file generation, health monitoring. **The agent handles judgment:** reading documentation, proposing rules, and presenting them for user approval. This separation means the agent can be Claude, Cursor, Copilot, or any LLM — the binary doesn't care.

### What gets proposed

| Category | Example |
|----------|---------|
| Migration footgun | Pydantic deprecated `schema()` — use `model_json_schema()` |
| Non-obvious default | SQLAlchemy `create_engine()` pools connections by default |
| Convention divergence | FastAPI docs say `async def`, most tutorials use `def` |
| Breaking change | API will fail in next major version |
| Semantic practice | Error messages should include the invalid value |

### What gets rejected

- Generic advice ("write clean code")
- Things standard linters already catch (ruff, biome, clippy)
- Subjective preferences without documentation backing
- Rules with no testable signal

## Commands

```bash
whetstone <command> [options]
```

| Command | Alias | Purpose | Key Flags |
|---------|-------|---------|-----------|
| `doctor` | — | One-command bootstrap | `--json`, `--full-run`, `--resume`, `--changed-only` |
| `status` | — | Project health summary | `--json`, `--score`, `--history`, `--no-drift-check` |
| `ci-check` | `check` | CI freshness check | `--json`, `--pr-comment`, `--fail-on`, `--changed-only` |
| `detect-deps` | `deps` | Detect dependencies | `--check-drift`, `--changed-only`, `--incremental` |
| `resolve-sources` | `resolve` | Resolve documentation URLs | `--changed-only`, `--force-refresh`, `--resume` |
| `generate-context` | `context` | Generate agent files | `--dry-run`, `--formats`, `--lang` |
| `generate-tests` | `tests` | Generate test + lint files | `--dry-run`, `--lang` |

All commands accept `--project-dir` (default: `.`) and output JSON to stdout. Human-readable progress goes to stderr. JSON responses include a `next_command` field suggesting what to run next.

> **Legacy Python scripts:** The `scripts/` directory contains the original Python implementations. These are maintained for reference but the Rust binary is the primary interface.

## Outputs

### Rule YAML files

Rules live in `whetstone/rules/{language}/{dependency}.yaml`:

```yaml
source:
  name: fastapi
  version: "0.115.0"
  content_hash: sha256:abc123

rules:
  - id: fastapi.async-routes
    severity: must            # must | should | may
    confidence: high          # high | medium
    category: convention      # migration | default | convention | breaking-change | semantic
    description: >
      Route handlers MUST use async def.
    source_url: https://fastapi.tiangolo.com/async/
    approved: true
    signals:
      - id: is-sync-function
        strategy: ast         # ast | pattern | lint_proxy | ai
        weight: required
```

See [`references/rule-schema.yaml`](references/rule-schema.yaml) for the full schema.

### Generated files

| Output | Location | Purpose |
|--------|----------|---------|
| Tests | `whetstone/tests/` | Native test files (pytest/vitest/cargo) |
| Lint configs | `whetstone/lint/` | Ruff/biome/clippy configuration fragments |
| Agent context | `AGENTS.md`, `CLAUDE.md`, `.cursorrules` | Instructions for AI coding agents |

### Status output

`whetstone status` returns a health score (0-100) with five dimensions:

| Dimension | What it measures |
|-----------|-----------------|
| `freshness_days` | Days since last rule extraction |
| `rules_count` | Total approved rules |
| `high_confidence_ratio` | % of rules with `confidence: high` |
| `deterministic_coverage` | % of signals using ast/pattern/lint_proxy (not ai) |
| `pending_updates` | Dependencies with version drift |

Labels: **Healthy**, **Needs Review**, **Stale**, **No Rules**.

### Impact metrics

`whetstone status` also includes a `metrics` object for tracking value over time:

| Metric | What it measures |
|--------|-----------------|
| `rules_approved` | Total approved rules |
| `rules_proposed` | Total rules proposed (including unapproved) |
| `approval_rate` | % of proposed rules that were approved |
| `must_rules` | Count of highest-severity (`must`) rules |
| `dependencies_covered` | Dependencies with at least one approved rule |
| `dependencies_total` | Total tracked dependencies |
| `dependency_coverage` | % of dependencies with rules |
| `deterministic_coverage` | % of signals using ast/pattern/lint_proxy |
| `pending_drift` | Dependencies with version drift |

### Metric history

Every `whetstone status` run automatically appends a timestamped snapshot to `whetstone/.metrics.jsonl`. Use `--history` to see trends:

```bash
whetstone status --history
```

This shows a table of score, label, rules count, and drift over time. Use `--no-snapshot` to skip recording (e.g., in scripts that poll status without wanting to inflate history).

**Anti-gaming guidance:** Metrics reflect the state of your rules, not your code quality. A high score with 5 well-chosen rules is better than a high score with 50 trivial rules. Focus on the `must_rules` and `deterministic_coverage` metrics — these indicate rules that catch real mistakes with real checks. The `approval_rate` metric helps calibrate extraction quality: if it's consistently low, your extraction prompt may need tuning.

## CI Integration

### GitHub Action

```yaml
# .github/workflows/whetstone.yml
name: Whetstone Check
on:
  pull_request:
    branches: [main]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: whetstone/whetstone@main
        id: whetstone
        with:
          changed-only: "true"
          fail-on: stale
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

Action inputs:

| Input | Default | Description |
|-------|---------|-------------|
| `directory` | `.` | Project directory to check |
| `changed-only` | `true` | Only check dependencies with version drift |
| `fail-on` | `none` | Exit with error on: `stale`, `needs_review`, or `none` |
| `github-token` | — | GitHub token for posting PR comments |
| `python-version` | `3.11` | Python version to use |

Action outputs: `freshness_status`, `changed_sources_count`, `recommended_rules_count`, `requires_review`, `score`.

## Languages

| Language | Manifest | Registry | Tests | Linter | Support Tier |
|----------|----------|----------|-------|--------|--------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff | **Full** — reference implementation |
| TypeScript | `package.json` | npm | vitest | biome | **Baseline** — common signals work, complex patterns scaffold |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy | **Baseline** — common signals work, complex patterns scaffold |

### What each tier means

**Full (Python):** AST-based checks for function signatures, decorators, imports, class inheritance, keyword arguments. Pattern-based checks for string literals and naming conventions. Ruff overlay generation for lint_proxy signals. Generated tests are complete and runnable.

**Baseline (TypeScript, Rust):** Pattern/string-matching checks for deprecated APIs, import statements, and common patterns. Generated tests work for these signal types. Complex AST patterns (e.g., type inference, trait bounds) produce TODO scaffolds that need manual implementation. Biome/clippy overlay generation works for lint_proxy signals.

## Privacy

Whetstone's `detect-patterns.py` mines agent conversation transcripts (Claude Code, Cursor, Cline, etc.) for recurring style patterns. **By default, transcript scanning is scoped to the current project only** — it filters transcripts by matching the project directory name in the file path.

This means Whetstone will NOT read conversations from unrelated projects unless you explicitly opt in.

| Mode | Behavior | Flag |
|------|----------|------|
| **Default (scoped)** | Only reads transcripts whose path contains the current project name | None needed |
| **Global** | Reads all agent transcripts across `$HOME` | `--global-transcripts` |

**What is read:** Only `user`/`human` role messages from JSONL transcript files. Agent responses are ignored. No transcript content is sent to any external service — all processing is local.

**What is stored:** Nothing from transcripts is persisted. Pattern results are ephemeral JSON output. The only file Whetstone writes is `whetstone/.last-run` (a timestamp).

**Directories scanned:** `~/.claude/projects`, `~/.cursor/projects`, `~/.cline/projects`, `~/.continue/sessions`, `~/.codex/sessions`, `~/.goose/sessions`, `~/.roo/projects`, `~/.agents/sessions`, `~/.config/opencode/sessions`, `~/.windsurf/sessions`.

If you're concerned about privacy, use the default scoped mode (no flag needed) or exclude transcript mining entirely with `--sources git,pr`.

## How Whetstone Fits with Existing Tools

Whetstone is designed to complement — not replace — your existing toolchain.

| Tool | What it does | How Whetstone complements it |
|------|-------------|------------------------------|
| **ruff / biome / clippy** | Enforces syntax, formatting, and general code quality rules | Whetstone catches dependency-specific practices these linters don't know about. Where a linter rule exists but isn't enabled, Whetstone generates a lint overlay to enable it. |
| **PR review bots** (reviewdog, danger, etc.) | Automated checks on pull requests | Whetstone generates the rules these bots enforce. Run `whetstone ci-check` in CI for freshness gating alongside your existing checks. |
| **AI code review** (CodeRabbit, Copilot review, etc.) | LLM-powered code review | Whetstone provides deterministic, source-backed rules that don't vary between runs. Use it for the checks you want to enforce consistently, AI review for everything else. |
| **AGENTS.md / .cursorrules** | Static agent instructions | Whetstone auto-generates and keeps these files current. When dependencies update, your agent instructions update too. |
| **Semgrep / CodeQL** | Custom static analysis rules | For TypeScript and Rust, Whetstone can generate signal patterns that map to Semgrep rules. For Python, Whetstone's pytest-based checks are simpler to maintain. |

### What Whetstone adds that nothing else does

1. **Source-backed provenance** — every rule cites a specific documentation URL
2. **Drift detection** — knows when your dependencies updated and your rules didn't
3. **Multi-output from single source** — same approved rule becomes a test, a lint config, and an agent instruction
4. **Recency awareness** — prioritizes rules about recent changes that LLMs weren't trained on

## FAQ

**How is this different from a linter?**
Linters enforce syntax and formatting rules. Whetstone catches dependency-specific practices that linters don't know about — migration footguns, non-obvious defaults, convention divergence. It generates linter config fragments where possible, and native tests for everything else.

**Do I need an LLM API key?**
No. Whetstone is an Agent Skill — the agent running it (Claude, Cursor, etc.) acts as the LLM. No separate API key or binary required.

**What if Whetstone doesn't find any rules for my dependency?**
That's correct behavior. If the documentation doesn't clearly state practices worth enforcing, Whetstone stays silent. You can always add rules manually.

**Can I add custom sources beyond dependency docs?**
The extraction prompt works with any documentation content — team style guides, blog posts, migration guides. Currently, you provide custom source content to the agent manually during extraction. *Planned: automated custom URL ingestion.*

**What happens if I don't install Whetstone?**
Nothing breaks. The generated tests, lint configs, and agent context files are standard files in your repo. They run with your existing CI and work with any agent that reads `AGENTS.md` or `.cursorrules`.

**How do I update rules when dependencies change?**
Run `whetstone status` or `whetstone ci-check` to see which dependencies have drifted. Then run `whetstone doctor --changed-only` to re-extract rules only for what changed.

**What's the `next_command` field in every output?**
Every script suggests what to do next. Agent clients can use this to chain commands automatically without reading documentation.

## Self-Hosting (Dogfooding)

Whetstone can be used on itself. The `tests/fixtures/` directory contains sample manifests that demonstrate the full workflow. To run the self-hosting workflow:

```bash
# Run doctor against the test fixtures
whetstone doctor --project-dir tests/fixtures --json

# Check status of existing rules
whetstone status --project-dir tests/fixtures

# Generate test artifacts from the sample rules
whetstone generate-tests --project-dir tests/fixtures --dry-run
whetstone generate-context --project-dir tests/fixtures --dry-run
```

The test fixtures include rule files for fastapi and react that demonstrate the full rule schema with lifecycle fields, provenance metadata, and golden examples. This serves as a reference for the quality bar Whetstone expects.

## Current Capabilities vs Roadmap

**Shipped today:**
- Dependency detection across Python, TypeScript, and Rust (including monorepos)
- Documentation resolution via registry APIs with llms.txt probing
- Agent-mediated rule extraction with structured approval flow
- Test generation (pytest, vitest, cargo test) and lint overlays (ruff, biome, clippy)
- Agent context generation (AGENTS.md, CLAUDE.md, .cursorrules, and 3 more formats)
- Health monitoring with drift detection, freshness scoring, and metric history
- CI integration via GitHub Action with PR comments
- Privacy-scoped transcript mining for style patterns

**Planned (not yet implemented):**
- AI eval runner for ambiguous signals (`check --ai-only`)
- Pattern detection from agent transcripts and git history
- Layer system (personal → project → team → built-in)
- Rule promotion across layers (`promote` command)
- Automated custom URL ingestion for non-registry sources
- Shared rule registry with community-ranked rules

See `planning/roadmap.md` for the full phased delivery plan.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `No manifests found` | Ensure `pyproject.toml`, `package.json`, or `Cargo.toml` exists in your project directory |
| `status: not_initialized` | Run `whetstone doctor` first to detect deps and create the `whetstone/` directory |
| Drift check is slow | Use `--no-drift-check` for faster status, or `--changed-only` to limit scope |
| Rules from stale docs | Check `source_url` in your rule YAML — Whetstone flags when source content changes via `content_hash` |

---

*Whetstone sharpens the tools that write your code.*
