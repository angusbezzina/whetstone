# Claude Code Instructions for Whetstone

> Whetstone sharpens the tools that write your code.

Read `AGENTS.md` for universal project context. This file contains Claude Code-specific instructions.

---

## Project Context

Whetstone is an Agent Skill (agentskills.io format) with a **Rust CLI binary** that derives coding rules from dependency documentation and developer patterns. The MVP architecture is documented in `planning/mvp.md`. The full vision is in `planning/product-spec.md`.

**The Rust binary (`src/`) is the sole runtime implementation.** Archived Python command implementations live under `scripts/legacy/` strictly as parity reference for `tests/test_script_contracts.py`. Pattern mining (`wh patterns`), rule-schema validation (`wh validate`), and every other user-facing workflow are Rust-native. The agent (you) acts as the LLM for rule extraction -- the binary handles deterministic work.

---

## Issue Tracking: Beads

Use **bd** (beads) for ALL planning and issue tracking unless the user explicitly says otherwise.

```bash
bd onboard            # Get started
bd ready              # Find available work
bd show <id>          # View details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd dolt push          # Push Beads data to the remote Dolt ref
bd dolt pull          # Pull Beads data from the remote Dolt ref
```

Do NOT use ad-hoc TODO comments, task lists, or other tracking as a substitute for beads. If the user asks to plan work, create beads. If there's follow-up work at session end, create beads.
Do NOT depend on the legacy `bd sync` / `beads-sync` workflow in this repo; the intended Beads setup is the current Dolt-native collaboration model documented upstream.
If local `.beads` state is broken or another machine is missing current issues, repair it with `./scripts/beads-repair.sh` instead of improvising ad hoc Beads/Dolt state surgery.

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

Archived Python reference scripts live under `scripts/legacy/`. There are no active top-level Python helpers -- all runtime commands ship from the Rust binary. If you are touching archived scripts (only for parity regression coverage), the rules below still apply:

- Target Python 3.10+ (use `match` statements, `|` union types where appropriate)
- Use only stdlib + `requests` + `pyyaml` + `toml` -- keep dependencies minimal
- Every archived script must still be runnable standalone when used for parity/reference checks
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
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test
   python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
   cargo run --quiet --release -- validate
   python3 -m pytest -q
   ```
   Do not push if Ruff fails. This exact command mirrors the CI gate that has been failing on import ordering issues.
3. Update bead status (close finished, update in-progress)
4. Push:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test
   python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
   cargo run --quiet --release -- validate
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
   If your local Beads setup supports Dolt remotes for this repo, run `bd dolt push` after bead updates and before ending the session.
5. Verify all changes are committed AND pushed
6. Hand off context for next session

**NEVER** stop before pushing. **NEVER** say "ready to push when you are". YOU push.

## Gates Must Pass Locally Before Every Push

**Non-negotiable.** CI mirrors these gates exactly. Pushing failing code costs the whole team — fix locally first. When ending a session, run the full suite and only push when every gate is green.

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
cargo run --quiet --release -- validate
python3 -m pytest -q
```

This repo ships a pre-push hook at `.githooks/pre-push` that runs all five gates and aborts the push on any failure. **Before your first push in any session, verify the hook is wired up** — `core.hooksPath` must be `.githooks`, and the hook must be executable.

### Preflight (run at the start of any coding session)

```bash
# If either line is needed, the hook wasn't active — likely why failing code shipped before.
test "$(git config core.hooksPath)" = ".githooks" || git config core.hooksPath .githooks
test -x .githooks/pre-push || chmod +x .githooks/pre-push
```

Never bypass with `--no-verify`. If a gate fails, fix the underlying issue — do not skip the hook, comment out the check, or delete tests to make gates pass.

---

## Project-Specific Patterns

### File Structure

```
whetstone/
├── SKILL.md                        # Agent skill (workflow + extraction prompt)
├── CLAUDE.md                       # This file
├── AGENTS.md                       # Universal agent instructions
├── Cargo.toml                      # Rust project manifest
├── src/                            # Rust source (primary implementation)
│   ├── main.rs                    # Entry point
│   ├── cli.rs                     # CLI argument parsing (clap)
│   ├── doctor.rs                  # One-command bootstrap orchestrator
│   ├── detect/                    # Dependency detection (Python/TS/Rust)
│   ├── resolve/                   # Source URL resolution + content fetching
│   ├── rules.rs                   # Structured YAML rule parsing + validation
│   ├── generate_context.rs        # Multi-format agent context generation
│   ├── generate_tests.rs          # Test + linter config generation
│   ├── status.rs                  # Health score, drift detection
│   ├── ci_check.rs                # CI freshness gating
│   ├── state/                     # State management (cache, inventory, manifests)
│   ├── config.rs                  # Config file loading
│   ├── output.rs                  # JSON/report formatting
│   └── types.rs                   # Shared type definitions
├── scripts/                        # Legacy Python scripts (reference only)
├── tests/                          # Rust integration tests + fixtures
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

---

## Release Protocol

Whetstone ships as a single self-contained binary via GitHub Releases. Agents
MUST follow this protocol when the user asks to cut a release.

### Pre-release checklist (all must pass)
```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
python3 -m ruff check scripts/ tests/ --select E,F,W,I --ignore E501
cargo run --quiet --release -- validate
python3 -m pytest -q
```

### Versioning
- Follow [Semantic Versioning](https://semver.org/): MAJOR.MINOR.PATCH
- `Cargo.toml` version field is the source of truth
- Git tags use `v` prefix: `v0.1.0`, `v0.2.0`, etc.
- Bump `Cargo.toml` version **before** tagging — the tag commit must contain the matching version

### Cutting a release
1. **Update CHANGELOG.md**: add a new `## [X.Y.Z] - YYYY-MM-DD` section with Added/Changed/Fixed/Removed subsections. Add a link reference at the bottom. Every user-visible change since the last release must be listed.
2. **Bump version** in `Cargo.toml`
3. **Commit**: `chore: release vX.Y.Z`
4. **Tag and push**:
   ```bash
   git tag vX.Y.Z
   git push && git push origin vX.Y.Z
   ```
5. **Wait for release.yml** to build binaries, validate, and create the GitHub Release
6. **Verify** the release page has 4 binaries + checksums-sha256.txt
7. **Test install.sh** from a clean directory:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh -s -- --version vX.Y.Z
   whetstone --version  # must print the new version
   ```
8. **Update Homebrew formula** (if tap is published): copy new sha256 values from checksums-sha256.txt into `packaging/homebrew/whetstone.rb`, bump version, push to tap repo

### What agents must NEVER do
- Tag a release without updating CHANGELOG.md and Cargo.toml version
- Push a tag that doesn't match the Cargo.toml version
- Skip the pre-release quality gates
- Delete or force-push a tag that has already been published as a GitHub Release

---

### Commit Message Style

Use conventional commits:
- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation changes
- `refactor:` for code restructuring
- `test:` for test additions/changes
- `chore:` for maintenance tasks
