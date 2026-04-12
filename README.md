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

### How Whetstone compares

| Tool | What it does | Whetstone's angle |
|------|-------------|-------------------|
| **Semgrep / CodeQL** | Custom SAST rules you write manually | Whetstone derives rules from docs — you review, not author |
| **Continue.dev** | AI code review from hand-written markdown rules | Whetstone generates the rules from dependency documentation |
| **CodeRabbit** | AI PR review (2M+ repos) | Reads Whetstone's output — `.cursorrules`, `CLAUDE.md`, `AGENTS.md` |
| **Ruff / Biome / Clippy** | Language-level linting | Whetstone catches dependency-specific rules they don't cover |

Whetstone is not a general AI code reviewer or a replacement for linters. It's the **rule-intelligence layer** — it decides which rules are worth enforcing, proves them with documentation, and generates enforcement artifacts for the tools you already use.

## Quick Start

**Prerequisites:** Rust toolchain (for building from source), or download a release binary. Git and internet access for registry lookups.

### Install

The recommended install path is `install.sh`, which downloads the latest
release binary for your platform and verifies its sha256 against the
published checksum file:

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
```

By default the binary is placed at `~/.local/bin/whetstone`. Override with
`INSTALL_DIR=/usr/local/bin` or similar. No repo checkout or Rust toolchain
is required on the target machine.

Alternatives:

```bash
# Homebrew (once the tap is published — see packaging/homebrew/README.md)
brew install angusbezzina/tap/whetstone

# From source with Cargo
cargo install --git https://github.com/angusbezzina/whetstone

# From a local checkout
cargo build --release && ./target/release/whetstone --help
```

`whetstone` is a single self-contained binary; once installed, `whetstone
doctor --project-dir <your-repo>` works from any directory — there is no
requirement to run it from inside the Whetstone checkout.

### Recommended repo setup for contributors

Enable the repo-managed pre-push hook so local pushes run the same quality gates used in CI:

```bash
git config core.hooksPath .githooks
chmod +x .githooks/pre-push
```

If your local Beads state gets out of sync or a new machine cannot see current
issues, repair/hydrate the local Beads Dolt database with:

```bash
./scripts/beads-repair.sh --role contributor
```

### Usage

```bash
# 1. Run the doctor — one command from zero to working rules
wh doctor
# → detects dependencies from pyproject.toml / package.json / Cargo.toml
# → resolves documentation URLs from registries, probes for llms.txt
# → outputs extraction context for the agent

# 2. The agent reads the doctor output, proposes rules, you approve each one

# 3. Generate tests and agent context from approved rules
wh context
wh tests
# → pytest/vitest/cargo test files in whetstone/evals/
# → lint overlays in whetstone/lint/
# → agent context files in whetstone/context/

# 4. Check project health anytime
wh status
```

> **Agent skill mode:** When using Whetstone as an agent skill, say "wh doctor" or "extract rules" and the agent runs the full workflow. The binary handles deterministic work; your existing LLM does the extraction.

### Worked Example: Extracting Rules for a Rust Project

Here's what a real run looks like on Whetstone's own codebase (Rust, 10 dependencies):

```bash
$ wh doctor
────────────────────────────────────────
  Whetstone Doctor — 2026-04-09
────────────────────────────────────────
  Dependencies: 16 runtime (+2 dev) across python, rust
  Sources:      10 resolved with content (README + changelog)
  Changelogs:   5 found (clap, chrono, rayon, reqwest, regex)
  Ready:        10 dependencies ready for extraction
────────────────────────────────────────
```

The agent reads the resolved content and proposes rules. Each rule is presented as a card:

```
[MUST] reqwest.set-timeout — high confidence — default
  Source kind: official_docs
  MUST set an explicit timeout on reqwest clients. Default is no timeout.
  Source: https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html
  Risk:   Hangs indefinitely on unresponsive servers.
  Signal: pattern — Client::new\s*\(\)  [match: regex]
  > Approve / Edit / Deny / Skip?
```

You approve or deny each rule. Approved rules are written to `whetstone/rules/rust/reqwest.yaml`.

Then generate outputs:

```bash
$ wh validate     # ✓ All schema checks passed
$ wh context      # → whetstone/context/AGENTS.md (11 rules, 302 lines)
$ wh tests        # → whetstone/evals/rust/test_reqwest.rs (real regex checks)
$ wh status       # → Score: 95 | Label: Healthy
```

The generated test for `reqwest.set-timeout` actually scans your source code:

```rust
let pattern = regex::Regex::new(r"Client::new\s*\(\)").unwrap();
for (line_num, line) in content.lines().enumerate() {
    if pattern.is_match(line) {
        violations.push(format!("{}:{}: {}", file.display(), line_num + 1, line.trim()));
    }
}
```

Run `cargo test` and the test catches any `Client::new()` calls without explicit timeouts. Meanwhile, `AGENTS.md` tells your AI coding agent to use timeouts from the start — enforcement before AND after code is written, from the same approved rule.

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
| `detect-patterns` | `patterns` | Mine style patterns from transcripts/git/PRs | `--sources`, `--since`, `--quiet`, `--global-transcripts` |

All commands accept `--project-dir` (default: `.`) and output JSON to stdout. Human-readable progress goes to stderr. JSON responses include a `next_command` field suggesting what to run next.

> **Python is not a runtime dependency.** Every user-facing command ships from the Rust binary. Archived Python reference implementations live under `scripts/legacy/` solely so `tests/test_script_contracts.py` can parity-test the Rust ports.

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

`wh status` returns a health score (0-100) with five dimensions:

| Dimension | What it measures |
|-----------|-----------------|
| `freshness_days` | Days since last rule extraction |
| `rules_count` | Total approved rules |
| `high_confidence_ratio` | % of rules with `confidence: high` |
| `deterministic_coverage` | % of signals using ast/pattern/lint_proxy (not ai) |
| `pending_updates` | Dependencies with version drift |

Labels: **Healthy**, **Needs Review**, **Stale**, **No Rules**.

### Impact metrics

`wh status` also includes a `metrics` object for tracking value over time:

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

Every `wh status` run automatically appends a timestamped snapshot to `whetstone/.metrics.jsonl`. Use `--history` to see trends:

```bash
wh status --history
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

#### Action migration note

Older Whetstone revisions used Python scripts inside the GitHub Action.
The current action builds and runs the Rust binary directly. If you previously
depended on Python internals, migrate to the documented action inputs/outputs in
`action.yml` instead of shelling out to a script.

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

Pattern mining from agent transcripts is an **opt-in** workflow exposed by `wh patterns`. Transcript scanning is scoped to the current project by default: only JSONL files whose path contains the current project directory name are read.

This means Whetstone will NOT read conversations from unrelated projects unless you explicitly opt in with `--global-transcripts`.

| Mode | Behavior | Flag |
|------|----------|------|
| **Default (scoped)** | Only reads transcripts whose path contains the current project name | None needed |
| **Global** | Reads all agent transcripts across `$HOME` | `--global-transcripts` |

**What is read:** Only `user`/`human` role messages from JSONL transcript files. Agent responses are ignored. No transcript content is sent to any external service — all processing is local.

**What is stored:** Nothing from transcripts is persisted. Pattern results are ephemeral JSON output. The only file `detect-patterns` writes is `whetstone/.last-run` (a timestamp used by `--since-last-run`).

**Directories scanned:** `~/.claude/projects`, `~/.cursor/projects`, `~/.cline/projects`, `~/.continue/sessions`, `~/.codex/sessions`, `~/.goose/sessions`, `~/.roo/projects`, `~/.agents/sessions`, `~/.config/opencode/sessions`, `~/.windsurf/sessions`.

If you're concerned about privacy, omit `detect-patterns` from your workflow (it is not run by `doctor` unless you explicitly pass `--sources`) or drop the `transcript` source with `--sources git,pr`.

## How Whetstone Fits with Existing Tools

Whetstone is designed to complement — not replace — your existing toolchain.

| Tool | What it does | How Whetstone complements it |
|------|-------------|------------------------------|
| **ruff / biome / clippy** | Enforces syntax, formatting, and general code quality rules | Whetstone catches dependency-specific practices these linters don't know about. Where a linter rule exists but isn't enabled, Whetstone generates a lint overlay to enable it. |
| **PR review bots** (reviewdog, danger, etc.) | Automated checks on pull requests | Whetstone generates the rules these bots enforce. Run `wh ci` in CI for freshness gating alongside your existing checks. |
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
Yes. Add any URL to `whetstone.yaml` and Whetstone fetches it alongside registry sources:

```yaml
sources:
  custom:
    - url: https://team-guide.internal/rust-conventions
      name: "Team Rust Guide"
      source_kind: team_guide
    - url: https://blog.example.com/fastapi-pitfalls
      name: "FastAPI Pitfalls"
      source_kind: blog
```

Custom sources appear in the doctor output for extraction. Each rule you extract from them gets tagged with `source_kind` for filtering.

**What happens if I don't install Whetstone?**
Nothing breaks. The generated tests, lint configs, and agent context files are standard files in your repo. They run with your existing CI and work with any agent that reads `AGENTS.md` or `.cursorrules`.

**How do I update rules when dependencies change?**
Run `wh status` or `wh ci` to see which dependencies have drifted. Then run `wh doctor --changed-only` to re-extract rules only for what changed.

**What's the `next_command` field in every output?**
Every script suggests what to do next. Agent clients can use this to chain commands automatically without reading documentation.

## Self-Hosting (Dogfooding)

Whetstone can be used on itself. The `tests/fixtures/` directory contains sample manifests that demonstrate the full workflow. To run the self-hosting workflow:

```bash
# Run doctor against the test fixtures
wh doctor --project-dir tests/fixtures --json

# Check status of existing rules
wh status --project-dir tests/fixtures

# Generate test artifacts from the sample rules
wh tests --project-dir tests/fixtures --dry-run
wh context --project-dir tests/fixtures --dry-run
```

The test fixtures include rule files for fastapi and react that demonstrate the full rule schema with lifecycle fields, provenance metadata, and golden examples. This serves as a reference for the quality bar Whetstone expects.

## Current Capabilities vs Roadmap

**Shipped today:**
- Dependency detection across Python, TypeScript, and Rust (including monorepos)
- 4-tier content resolution: llms.txt → registry README → HTML docs → GitHub changelog
- Changelog fetching with 18-month recency filtering
- Custom source URLs in `whetstone.yaml` (blogs, team guides, any public URL)
- 5 built-in Rust rules (`whetstone:recommended`) that ship with the binary
- Agent-mediated rule extraction with structured approval flow
- Test generation with real regex checks (via `match` field on signals)
- Lint overlay generation (ruff, biome, clippy)
- Agent context generation (AGENTS.md, CLAUDE.md, .cursorrules, copilot, windsurf, codex)
- Source attribution: `content_origin` (how fetched) + `source_kind` (what kind of source)
- Health monitoring with drift detection, freshness scoring, and metric history
- CI integration via GitHub Action with PR comments
- Drift-based refresh command (`wh refresh --check`)

**Opt-in:**
- Pattern detection from agent transcripts, git history, and PR comments via `wh patterns`.

**Planned:**
- ast-grep pattern generation (structural enforcement via CodeRabbit-compatible rules)
- MCP server for agent-native rule queries
- Continue.dev check generation for CI status checks
- AI eval runner with threshold gating and calibration
- Layer system (personal → project → team → built-in)
- Shared rule registry with community-ranked rules

See `planning/whetstone-roadmap-v2.md` for the full plan.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `No manifests found` | Ensure `pyproject.toml`, `package.json`, or `Cargo.toml` exists in your project directory |
| `status: not_initialized` | Run `wh doctor` first to detect deps and create the `whetstone/` directory |
| Drift check is slow | Use `--no-drift-check` for faster status, or `--changed-only` to limit scope |
| Rules from stale docs | Check `source_url` in your rule YAML — Whetstone flags when source content changes via `content_hash` |

---

*Whetstone sharpens the tools that write your code.*
