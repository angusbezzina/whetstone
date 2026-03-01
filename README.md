# Whetstone

> Sharpen the tools that write your code.

Whetstone derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files. When your dependencies evolve, Whetstone tells you exactly what changed and proposes specific new rules.

It's a codegen tool, not a runtime dependency. A teammate who never installs Whetstone still gets every rule enforced through standard CI and every agent guided by current instructions.

## Why Whetstone?

**Rules go stale.** Linter configs and coding conventions are written once at project setup. Dependencies ship new versions, deprecate APIs, and introduce better patterns. Nobody updates the rules. Agents keep writing code against outdated practices.

**Semantic best practices are unenforced.** "Error messages should be actionable," "prefer composition over inheritance" — everyone agrees on these, no tool checks them.

**Agents aren't told what they need to know.** `AGENTS.md` and `.cursorrules` are written once by hand — if they're written at all — and never updated.

Whetstone solves all three. It treats documentation as a living source of truth, converts it into enforceable checks, and keeps everything current as your dependencies evolve.

### High confidence or silence

5 rules you trust completely beats 50 you have to review. Whetstone only proposes rules backed by specific documentation with deterministic signals. A project with 40 dependencies might get rules for 8 of them — those are the 8 that have something worth enforcing.

## Quick Start

**Prerequisites:** Python 3.9+, git, internet access for registry lookups.

```bash
# 1. Install (or clone) Whetstone as an agent skill
git clone https://github.com/yourusername/whetstone.git
pip install pyyaml  # required dependency

# 2. Run the doctor — one command from zero to working rules
python3 whetstone/scripts/doctor.py --project-dir .

# 3. The doctor detects dependencies, resolves their docs, and
#    outputs extraction context. Feed this to your agent to extract rules.
#    The agent proposes rules; you approve each one.

# 4. Generate tests and agent context from approved rules
python3 whetstone/scripts/generate-tests.py --project-dir .
python3 whetstone/scripts/generate-agent-context.py --project-dir .

# 5. Check project health anytime
python3 whetstone/scripts/status.py --project-dir .
```

The doctor is the recommended entry point — it chains dependency detection, source resolution, and pattern mining into a single flow.

## How It Works

```
Detect  →  Your manifests (pyproject.toml, package.json, Cargo.toml)
              are scanned for dependencies

Resolve →  Documentation URLs are resolved from package registries
              (PyPI, npm, crates.io), probing for llms.txt first

Extract →  An LLM reads the docs and proposes high-confidence rules:
              migration footguns, non-obvious defaults, convention
              divergence, breaking changes, semantic practices

Approve →  You review each rule. Whetstone never auto-approves.
              Every rule cites a specific documentation URL.

Generate → Approved rules produce:
              - Native tests (pytest / vitest / cargo test)
              - Lint configs (ruff / biome / clippy)
              - Agent context (AGENTS.md, CLAUDE.md, .cursorrules)

Monitor →  Whetstone detects version drift and tells you exactly
              which deps changed and what new rules to consider
```

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

| Script | Purpose | Key Flags |
|--------|---------|-----------|
| `doctor.py` | One-command bootstrap | `--json`, `--skip-patterns` |
| `status.py` | Project health summary | `--json`, `--score`, `--no-drift-check` |
| `ci-check.py` | CI freshness check | `--json`, `--pr-comment`, `--fail-on`, `--changed-only` |
| `detect-deps.py` | Detect dependencies | `--check-drift`, `--changed-only` |
| `resolve-sources.py` | Resolve documentation URLs | `--changed-only` |
| `detect-patterns.py` | Mine style patterns | `--sources` (transcripts, git, pr) |
| `generate-agent-context.py` | Generate agent files | `--dry-run` |
| `generate-tests.py` | Generate test + lint files | `--dry-run` |

All scripts accept `--project-dir` (default: `.`) and output JSON to stdout. Human-readable progress goes to stderr. Every JSON response includes a `next_command` field suggesting what to run next.

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

`status.py` returns a health score (0-100) with five dimensions:

| Dimension | What it measures |
|-----------|-----------------|
| `freshness_days` | Days since last rule extraction |
| `rules_count` | Total approved rules |
| `high_confidence_ratio` | % of rules with `confidence: high` |
| `deterministic_coverage` | % of signals using ast/pattern/lint_proxy (not ai) |
| `pending_updates` | Dependencies with version drift |

Labels: **Healthy**, **Needs Review**, **Stale**, **No Rules**.

### Impact metrics

`status.py` also includes a `metrics` object for tracking value over time:

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

These are derived from current state — no persistent storage needed. Track them over time to measure Whetstone's value to your team.

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

| Language | Manifest | Registry | Tests | Linter |
|----------|----------|----------|-------|--------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff |
| TypeScript | `package.json` | npm | vitest | biome |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy |

## FAQ

**How is this different from a linter?**
Linters enforce syntax and formatting rules. Whetstone catches dependency-specific practices that linters don't know about — migration footguns, non-obvious defaults, convention divergence. It generates linter config fragments where possible, and native tests for everything else.

**Do I need an LLM API key?**
No. Whetstone is an Agent Skill — the agent running it (Claude, Cursor, etc.) acts as the LLM. No separate API key or binary required.

**What if Whetstone doesn't find any rules for my dependency?**
That's correct behavior. If the documentation doesn't clearly state practices worth enforcing, Whetstone stays silent. You can always add rules manually.

**Can I add custom sources beyond dependency docs?**
Yes. You can point Whetstone at any URL — team style guides, blog posts, migration guides. It extracts rules from whatever sources you trust.

**What happens if I don't install Whetstone?**
Nothing breaks. The generated tests, lint configs, and agent context files are standard files in your repo. They run with your existing CI and work with any agent that reads `AGENTS.md` or `.cursorrules`.

**How do I update rules when dependencies change?**
Run `status.py` or `ci-check.py` to see which dependencies have drifted. Then run the doctor or resolve-sources with `--changed-only` to re-extract rules only for what changed.

**What's the `next_command` field in every output?**
Every script suggests what to do next. Agent clients can use this to chain commands automatically without reading documentation.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `No manifests found` | Ensure `pyproject.toml`, `package.json`, or `Cargo.toml` exists in your project directory |
| `ModuleNotFoundError: yaml` | Run `pip install pyyaml` — it's the only required dependency |
| `status: not_initialized` | Run `doctor.py` first to detect deps and create the `whetstone/` directory |
| Drift check is slow | Use `--no-drift-check` for faster status, or `--changed-only` to limit scope |
| Rules from stale docs | Check `source_url` in your rule YAML — Whetstone flags when source content changes via `content_hash` |

---

*Whetstone sharpens the tools that write your code.*
