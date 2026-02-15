# Agent Instructions for Whetstone

> Whetstone sharpens the tools that write your code. It derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files.

## Project Overview

Whetstone is an Agent Skill (agentskills.io format) with Python helper scripts. The agent acts as the LLM for rule extraction -- no separate API key or binary required. Scripts handle deterministic work (dep detection, URL resolution, file generation). See `planning/mvp.md` for the full architecture.

**Key files:**
- `planning/one-pager.md` -- Product vision and positioning
- `planning/product-spec.md` -- Full technical specification
- `planning/roadmap.md` -- Phased delivery plan
- `planning/mvp.md` -- MVP build plan (current focus)

**MVP deliverables:**
- `SKILL.md` -- Core agent skill (workflow + extraction prompt)
- `scripts/detect-deps.py` -- Dependency detection (Python/TS/Rust)
- `scripts/resolve-sources.py` -- Source URL resolution + content fetching
- `scripts/detect-patterns.py` -- Mine transcripts/git/PRs for style patterns
- `scripts/generate-agent-context.py` -- Multi-format agent context generation
- `scripts/generate-tests.py` -- Test + linter config generation

---

## Issue Tracking with Beads

This project uses **bd** (beads) for ALL planning and issue tracking unless the user explicitly requests otherwise.

```bash
bd onboard            # Get started with beads
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git
```

**Rules:**
- ALWAYS use `bd` to create, track, and close issues
- ALWAYS run `bd sync` before pushing
- If the user asks to plan work, create beads for it -- do not use ad-hoc notes or TODO comments as a substitute for proper issue tracking
- Only skip beads if the user explicitly says to

---

## Researching Best Practices and Documentation

When searching for best practices, dependency documentation, API patterns, or any technical guidance:

1. **Get the current date first** -- run `date` or check the system clock before searching
2. **Search for the latest results** -- always scope searches to the current year and recent prior year. Documentation from 2+ years ago may reference deprecated APIs or outdated patterns
3. **Prefer primary sources** -- official documentation, `llms.txt` files, changelogs, and migration guides over blog posts or tutorials
4. **Verify currency** -- check that any referenced API, pattern, or configuration still exists in the current version of the dependency
5. **Flag stale sources** -- if you find documentation that appears outdated relative to the current dependency version, note this explicitly

This is critical because Whetstone's entire value proposition is keeping rules current. An agent that references stale documentation undermines the product.

---

## Core Philosophy: High Confidence or Silence

Whetstone is not a linter. It catches the things that matter most and that nothing else catches. Every decision should be guided by these principles:

- **5 rules you trust completely beats 50 you have to review**
- Every proposed rule MUST have at least one deterministic signal (AST or pattern check)
- Maximum 5 rules per dependency -- if you can't rank them, you haven't filtered hard enough
- Don't propose rules that standard linters already enforce (ruff, biome, clippy)
- Every rule must cite a specific URL in the dependency's documentation
- If you're not 90%+ confident a rule prevents a real mistake, don't propose it

### What gets rejected
- Generic advice ("write clean code", "use meaningful names")
- Things linters already catch
- Subjective preferences without source backing
- Rules with no testable signal
- Architecture principles that can't be decomposed into checks

### What gets accepted
- Migration footguns (deprecated APIs that still work)
- Non-obvious defaults (insecure/slow unless configured)
- Convention divergence (docs say X, most tutorials/LLMs default to Y)
- Breaking change preparation (will fail in next major version)
- Semantic practices decomposable into mostly-deterministic signals

---

## Languages and Ecosystems

Whetstone supports three language ecosystems:

| Language   | Manifest                          | Registry   | Test Framework | Linter |
|------------|-----------------------------------|------------|----------------|--------|
| Python     | `pyproject.toml`, `requirements.txt` | PyPI       | pytest         | ruff   |
| TypeScript | `package.json`                    | npm        | vitest         | biome  |
| Rust       | `Cargo.toml`                      | crates.io  | cargo test     | clippy |

---

## Rule YAML Format

Rules follow a strict schema. See `references/rule-schema.yaml` for the full specification. Key fields:

```yaml
- id: fastapi.async-routes
  severity: must              # must | should | may
  confidence: high            # high | medium
  category: convention        # migration | default | convention | breaking-change | semantic
  description: >
    Route handlers MUST use async def.
  source_url: https://fastapi.tiangolo.com/async/
  approved: true
  signals:
    - id: is-sync-function
      strategy: ast           # ast | pattern | lint_proxy | ai
      description: Function decorated with route decorator uses def instead of async def
      weight: required
  golden_examples:
    - code: |
        @app.get("/users")
        async def get_users(): ...
      verdict: pass
    - code: |
        @app.get("/users")
        def get_users(): ...
      verdict: fail
```

---

## Session Completion Protocol

When ending a work session, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

1. **File issues for remaining work** -- create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) -- tests, linters, type checks
3. **Update issue status** -- close finished beads, update in-progress items
4. **PUSH TO REMOTE**:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** -- clear stashes, prune remote branches
6. **Verify** -- all changes committed AND pushed
7. **Hand off** -- provide context for next session

**CRITICAL:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing -- that leaves work stranded locally
- NEVER say "ready to push when you are" -- YOU must push
- If push fails, resolve and retry until it succeeds
