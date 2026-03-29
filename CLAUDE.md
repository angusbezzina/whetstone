# Claude Code Instructions for Whetstone

> Whetstone sharpens the tools that write your code.

Read `AGENTS.md` for universal project context. This file contains Claude Code-specific instructions.

---

## Project Context

Whetstone is an Agent Skill (agentskills.io format) with Python helper scripts that derives coding rules from dependency documentation and developer patterns. The MVP architecture is documented in `planning/mvp.md`. The full vision is in `planning/product-spec.md`.

**This is a greenfield project** being built as a skill + scripts, not a compiled binary. The agent (you) acts as the LLM for rule extraction -- the scripts handle deterministic work.

---

## Issue Tracking: Beads

Use **bd** (beads) for ALL planning and issue tracking unless the user explicitly says otherwise.

```bash
bd onboard            # Get started
bd ready              # Find available work
bd show <id>          # View details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git (ALWAYS before pushing)
```

Do NOT use ad-hoc TODO comments, task lists, or other tracking as a substitute for beads. If the user asks to plan work, create beads. If there's follow-up work at session end, create beads.

---

## Researching Best Practices and Documentation

**CRITICAL**: Whetstone's value depends on rules being current. Stale documentation = bad rules.

When searching for best practices, dependency docs, API patterns, or technical guidance:

1. **Get the current date first** -- run `date` or check the system clock before any search
2. **Search for the latest results** -- scope searches to the current year and prior year. Documentation from 2+ years ago likely references deprecated APIs or outdated patterns
3. **Prefer primary sources** -- official docs, `llms.txt` files, changelogs, migration guides. Not blog posts or tutorials
4. **Verify currency** -- confirm any referenced API, pattern, or config still exists in the current version
5. **Flag stale sources** -- if documentation appears outdated relative to the current dependency version, call it out explicitly

---

## Writing Code

### Python Scripts

The `scripts/` directory contains Python helper scripts. When writing or modifying them:

- Target Python 3.10+ (use `match` statements, `|` union types where appropriate)
- Use only stdlib + `requests` + `pyyaml` + `toml` -- keep dependencies minimal
- Every script must be runnable standalone: `python3 scripts/scriptname.py`
- Every script outputs JSON to stdout for the agent to consume
- Use `argparse` for CLI flags
- Handle errors gracefully -- print JSON with an `"error"` key, don't crash with tracebacks
- Include a `if __name__ == "__main__":` block

### SKILL.md

The SKILL.md follows the agentskills.io specification:
- YAML frontmatter with `name`, `description` (required), plus optional `license`, `compatibility`, `metadata`
- The `name` field must match the parent directory name
- Body is markdown with workflow instructions
- Keep under 500 lines -- move detailed reference material to `references/`
- Reference scripts and files with relative paths from the skill root

### Rule YAML Files

Rules follow the schema in `references/rule-schema.yaml`. Key constraints:
- Every rule needs an `id`, `severity`, `confidence`, `category`, `description`, `source_url`
- Every rule needs at least one signal with `strategy: ast` or `strategy: pattern`
- Every rule needs 3-5 golden examples (mix of pass and fail)
- Maximum 5 rules per dependency

---

## Core Philosophy: High Confidence or Silence

This is the most important principle in the project. When proposing rules, generating code, or making design decisions:

- **5 rules you trust completely beats 50 you have to review**
- If you're not 90%+ confident, don't propose it
- Every rule MUST have at least one deterministic signal
- Don't duplicate what ruff, biome, or clippy already catch
- Every rule must cite a specific documentation URL
- "I'm not sure" is always better than a low-confidence rule

### Rule categories (only these are valid)
- `migration` -- deprecated APIs that still work but shouldn't be used
- `default` -- things that are insecure/slow unless configured
- `convention` -- docs recommend X, but most tutorials/LLMs default to Y
- `breaking-change` -- patterns that will fail in the next major version
- `semantic` -- practices requiring judgment, decomposable into mostly-deterministic signals

---

## Languages and Ecosystems

| Language   | Manifest                             | Registry   | Test Output  | Lint Output     |
|------------|--------------------------------------|------------|--------------|-----------------|
| Python     | `pyproject.toml`, `requirements.txt` | PyPI       | pytest       | ruff.toml       |
| TypeScript | `package.json`                       | npm        | vitest       | biome.json      |
| Rust       | `Cargo.toml`                         | crates.io  | cargo test   | clippy.toml     |

---

## Session Completion Protocol

When ending a work session, complete ALL steps. Work is NOT complete until `git push` succeeds.

1. File remaining work as beads
2. Run quality gates (if code changed)
   ```bash
   python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
   python3 -m pytest -q
   ```
   Do not push if Ruff fails. This exact command mirrors the CI gate that has been failing on import ordering issues.
3. Update bead status (close finished, update in-progress)
4. Push:
   ```bash
   python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. Verify all changes are committed AND pushed
6. Hand off context for next session

**NEVER** stop before pushing. **NEVER** say "ready to push when you are". YOU push.

## Git Hooks

This repo uses a repo-managed pre-push hook in `.githooks/pre-push` to run the Ruff gate locally before pushes.

One-time setup:

```bash
git config core.hooksPath .githooks
chmod +x .githooks/pre-push
```

---

## Project-Specific Patterns

### File Structure

```
whetstone/
├── SKILL.md                        # Agent skill (workflow + extraction prompt)
├── CLAUDE.md                       # This file
├── AGENTS.md                       # Universal agent instructions
├── scripts/
│   ├── detect-deps.py             # Dep detection (Python/TS/Rust)
│   ├── resolve-sources.py         # Source URL resolution + fetching
│   ├── detect-patterns.py         # Mine transcripts/git/PRs for patterns
│   ├── generate-agent-context.py  # Multi-format agent context generation
│   └── generate-tests.py          # Test + linter config generation
├── references/
│   ├── rule-schema.yaml           # Rule YAML format spec
│   ├── extraction-prompt.md       # The extraction prompt
│   └── signal-strategies.md       # Signal decomposition guide
├── assets/
│   └── whetstone.yaml.template    # Default config template
└── planning/
    ├── one-pager.md               # Product vision
    ├── product-spec.md            # Full technical spec
    ├── roadmap.md                 # Phased delivery plan
    └── mvp.md                     # MVP build plan (current focus)
```

### Naming Conventions

- Script files: `kebab-case.py` (e.g., `detect-deps.py`)
- Rule IDs: `dependency.rule-name` (e.g., `fastapi.async-routes`)
- Rule files: `dependency-name.yaml` (e.g., `fastapi.yaml`)
- Test files: `test_dependency_rule_name.py` (Python), `dependency-rule-name.test.ts` (TS), `whetstone_dependency_tests.rs` (Rust)

### Commit Message Style

Use conventional commits:
- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation changes
- `refactor:` for code restructuring
- `test:` for test additions/changes
- `chore:` for maintenance tasks
