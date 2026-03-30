# Whetstone MVP: Agent Skill + Scripts

> **Historical planning document.** This file describes the original Python-heavy MVP architecture. The current primary product/runtime is the Rust binary in `src/`. See `planning/cutover.md`, `planning/cutover-criteria.md`, and `planning/mvp-v2.md` for the current state.

> **Status: COMPLETE.** The MVP is fully implemented and shipped. See `planning/mvp-v2.md` for the current work phase focused on product sharpness and quality.

> Build the core Whetstone experience as an Agent Skill with Python helper scripts. Deliverable in one focused day.

---

## Architecture

The MVP is an **Agent Skill** (following the [agentskills.io](https://agentskills.io) format) with Python helper scripts. Any skills-compatible agent (Claude Code, Cursor, Codex, OpenCode, Goose, Roo, AMP, etc.) can install it. The agent itself acts as the LLM for rule extraction — no separate API key or binary required.

```
┌─────────────────────────────────────────────────────┐
│  Agent (Claude Code, Cursor, Codex, etc.)           │
│                                                     │
│  ┌─────────────┐   ┌───────────────────────────┐   │
│  │  SKILL.md   │   │  Python Scripts            │   │
│  │  (workflow   │──▶│  detect-deps.py            │   │
│  │   + prompt)  │   │  resolve-sources.py        │   │
│  │             │   │  detect-patterns.py        │   │
│  │             │   │  generate-agent-context.py  │   │
│  │             │   │  generate-tests.py          │   │
│  └─────────────┘   └───────────────────────────┘   │
│         │                       │                   │
│         ▼                       ▼                   │
│  Agent intelligence       Deterministic outputs     │
│  (extraction, review)     (files, configs, tests)   │
└─────────────────────────────────────────────────────┘
```

### Why This Architecture

- **No separate LLM client** — the agent IS the LLM. The skill teaches it the extraction prompt.
- **No binary to install** — `npx skills add` and you're running.
- **Agent-agnostic** — works with any skills-compatible agent.
- **Scripts handle the boring parts** — dep detection, URL resolution, file generation are deterministic Python.
- **Transition-friendly** — the YAML rule format and file structure carry over to a future Rust CLI.

---

## Trigger Modes

Users choose when Whetstone runs. Configured in `whetstone.yaml`:

```yaml
trigger:
  mode: manual          # manual | session | post-merge | scheduled
  schedule: "weekly"    # Only for scheduled mode: daily | weekly | biweekly | monthly
  auto_detect_patterns: true   # Run detect-patterns in background on session start
```

### Manual (Default)

User invokes the workflow explicitly by asking the agent:

```
"Run whetstone init"
"Extract rules for this project"
"Update my whetstone rules"
```

Best for: Getting started, one-off refreshes, full control.

### Session Start Hook

A Claude Code hook that runs `detect-patterns.py` asynchronously on every session start. If new patterns are detected since the last run, surfaces a brief summary to the agent context:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup",
        "hooks": [
          {
            "type": "command",
            "command": "python3 \"$CLAUDE_PROJECT_DIR\"/whetstone/scripts/detect-patterns.py --since-last-run --quiet",
            "async": true,
            "statusMessage": "Whetstone: checking for new style patterns..."
          }
        ]
      }
    ]
  }
}
```

Best for: Passive discovery of style patterns without manual intervention.

### Post-Merge Hook

A git `post-merge` hook that runs a lightweight freshness check after pulling changes. Detects dependency version changes and flags if rules may need updating:

```bash
#!/bin/bash
# .git/hooks/post-merge
python3 whetstone/scripts/detect-deps.py --check-drift
```

Alternatively, a Claude Code `PostToolUse` hook on Bash that triggers when `git pull` or `git merge` completes:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "\"$CLAUDE_PROJECT_DIR\"/whetstone/scripts/check-drift.sh"
          }
        ]
      }
    ]
  }
}
```

Best for: Teams that want rules to stay current with dependency changes.

### Scheduled

A cron job or CI scheduled workflow that runs the full extract+generate cycle periodically:

```yaml
# .github/workflows/whetstone.yml
on:
  schedule:
    - cron: '0 9 * * 1'  # Every Monday at 9am
jobs:
  whetstone-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: python3 whetstone/scripts/detect-deps.py --check-drift
      - run: python3 whetstone/scripts/detect-patterns.py --since "7 days ago"
```

Best for: Teams that want periodic nudges without manual runs.

---

## What Gets Built

### 1. SKILL.md — Core Agent Skill

The skill file that teaches any compatible agent the Whetstone workflow.

**Contents:**

- **Activation triggers**: When to activate (user says "whetstone", "extract rules", "update standards", etc.)
- **Workflow steps**: Init → Extract → Generate, with branching for updates
- **Extraction prompt**: The carefully crafted, high-confidence-or-silence prompt with:
  - Category filtering (migration, defaults, convention, breaking-change, semantic)
  - Signal decomposition requirements (every rule needs ≥1 deterministic signal)
  - Golden example requirements (3-5 pass/fail examples per rule)
  - 5-rule-per-dependency ceiling
  - Anti-noise filtering (reject generic advice, things linters catch, subjective preferences)
- **Rule YAML format**: The schema the agent outputs for each approved rule
- **Agent context templates**: How to format rules into CLAUDE.md, AGENTS.md, .cursorrules
- **Interactive approval protocol**: How to present rules for user review

**Key design principle:** The SKILL.md is the "brain" — it contains the judgment and instructions. Scripts are the "hands" — they handle file I/O and API calls.

### 2. `scripts/detect-deps.py` — Dependency Detection

Reads manifest files and outputs structured JSON.

**Supports:**
- `pyproject.toml` (PEP 621 + Poetry)
- `requirements.txt`
- `package.json`
- `Cargo.toml`

**Output format:**
```json
{
  "languages": ["python", "typescript", "rust"],
  "dependencies": [
    {
      "name": "fastapi",
      "version": ">=0.115.0",
      "language": "python",
      "dev": false,
      "manifest": "pyproject.toml"
    },
    {
      "name": "next",
      "version": "^15.0.0",
      "language": "typescript",
      "dev": false,
      "manifest": "package.json"
    },
    {
      "name": "tokio",
      "version": "1.42",
      "language": "rust",
      "dev": false,
      "manifest": "Cargo.toml"
    }
  ]
}
```

### 3. `scripts/resolve-sources.py` — Source Resolution + Fetching

Takes dependency list, resolves documentation URLs, fetches content.

**Resolution chain per language:**

| Language   | Registry API                                | Docs URL extraction                     | llms.txt probe                              |
|------------|---------------------------------------------|-----------------------------------------|---------------------------------------------|
| Python     | `https://pypi.org/pypi/{name}/json`         | `info.project_urls` or `info.home_page` | `{docs_url}/llms.txt`, `{docs_url}/llms-full.txt` |
| TypeScript | `https://registry.npmjs.org/{name}`         | `homepage` field                        | `{homepage}/llms.txt`                       |
| Rust       | `https://crates.io/api/v1/crates/{name}`    | `homepage`, `documentation`, `repository` | `https://docs.rs/{name}/latest/llms.txt`    |

**Priority order:**
1. `llms-full.txt` (most complete, structured for LLMs)
2. `llms.txt` (concise, structured)
3. Changelog/migration guide pages
4. Full docs URL (most expensive, last resort)

**Output format:**
```json
{
  "sources": [
    {
      "name": "fastapi",
      "language": "python",
      "docs_url": "https://fastapi.tiangolo.com",
      "llms_txt_url": "https://fastapi.tiangolo.com/llms.txt",
      "source_type": "llms_txt",
      "content": "...",
      "content_hash": "a3f2c8e1..."
    }
  ]
}
```

### 4. `scripts/detect-patterns.py` — Style Pattern Detection

Mines three data sources for recurring style/convention requests. This is the "learn from how you already work" feature.

**Source 1: Agent Conversation Transcripts**

Parses Claude Code `.jsonl` transcript files at `~/.claude/projects/{project}/`.

Looks for user messages containing style signals:
- Directive phrases: "always use", "prefer X over Y", "don't use", "make sure you", "never do"
- Style keywords: "format", "style", "convention", "naming", "pattern", "approach"
- Correction patterns: "that's not how we do it", "we use X here", "change this to"
- Version-specific: "use the new", "v2 way", "latest API", "deprecated"

Groups by semantic similarity and frequency. A pattern mentioned across 3+ separate sessions is high confidence.

**Source 2: Git History**

Analyzes recent commits and diffs:
- Commit messages matching style patterns: "fix style", "format", "lint", "convention", "refactor: rename"
- Repeated mechanical changes in diffs (e.g., consistent `var` → `const`, quote style changes, import ordering)
- Files frequently modified within hours of creation (post-generation fixup signal)
- `.eslintrc`/`ruff.toml`/`rustfmt.toml`/`biome.json` change history (explicit style preference evolution)

**Source 3: GitHub PR Review Comments** (optional, requires `gh` CLI)

Fetches recent closed PR review comments:
- Filters for comments containing style/convention language
- Groups by reviewer + pattern (same reviewer giving the same feedback = high signal)
- Extracts the "what to do instead" from the comment

**Output format:**
```json
{
  "patterns": [
    {
      "description": "Prefer early returns over nested conditionals",
      "source": "transcript",
      "occurrences": 7,
      "confidence": "high",
      "sessions": ["abc123", "def456", "ghi789"],
      "example_quotes": [
        "always use early returns instead of nesting",
        "can you refactor this to use guard clauses"
      ],
      "suggested_rule": {
        "description": "Functions SHOULD use early returns (guard clauses) instead of deeply nested conditionals",
        "severity": "should",
        "category": "convention",
        "signals": [
          {
            "strategy": "ast",
            "description": "Function has if/else nesting depth > 3"
          }
        ]
      }
    }
  ]
}
```

**Flags:**
- `--since-last-run` — only analyze data since last execution (for session hooks)
- `--since "7 days ago"` — time-bounded analysis
- `--quiet` — only output if new patterns found (for background runs)
- `--sources transcript,git,pr` — select which sources to mine

### 5. `scripts/generate-agent-context.py` — Agent Context Generation

Reads approved rules from `whetstone/rules/*.yaml` and generates agent context files.

**Supported formats:**
- `CLAUDE.md` — Claude Code instructions
- `AGENTS.md` — Universal agent instructions (agentskills.io format)
- `.cursorrules` — Cursor instructions
- `.github/copilot-instructions.md` — GitHub Copilot
- `.windsurfrules` — Windsurf
- `codex.md` — OpenAI Codex

**Output structure (per format):**

```markdown
# Project Coding Standards (Auto-generated by Whetstone)
# Last updated: 2026-02-14
# Source: whetstone/rules/*.yaml

## Patterns to USE

### FastAPI: Use async route handlers
Route handlers MUST use `async def`. Sync handlers block the event loop.
Source: https://fastapi.tiangolo.com/async/#in-a-hurry

✅ Do:
\`\`\`python
@app.get("/users")
async def get_users():
    return await db.fetch_users()
\`\`\`

❌ Don't:
\`\`\`python
@app.get("/users")
def get_users():
    return db.fetch_users()
\`\`\`

## Patterns to AVOID

### Pydantic: Don't use deprecated .schema()
Use `model_json_schema()` instead of `.schema()`. The latter is removed in v2.
Source: https://docs.pydantic.dev/latest/migration/

## Conventions
...
```

**Configurable in `whetstone.yaml`:**
```yaml
agents:
  - claude.md
  - agents.md
  - cursorrules
  - copilot-instructions.md
```

### 6. `scripts/generate-tests.py` — Test File Generation

Reads approved rules and generates native test files and linter configurations.

**Python (pytest):**
- AST signal rules → `test_*.py` files using Python's `ast` module
- Pattern signal rules → `test_*.py` files using `re` module
- Lint proxy rules → `ruff.whetstone.toml` overlay

**TypeScript (vitest):**
- AST signal rules → `*.test.ts` files using TypeScript compiler API or regex
- Pattern signal rules → `*.test.ts` files using regex
- Lint proxy rules → `biome.whetstone.json` overlay

**Rust (cargo test):**
- AST signal rules → `tests/whetstone_*.rs` files using string/regex matching
- Pattern signal rules → same, with `regex` crate
- Lint proxy rules → clippy configuration in `Cargo.toml` `[lints.clippy]`

**Example generated test (Python):**
```python
# whetstone/evals/python/test_fastapi_async_routes.py
# Rule: fastapi.async-routes
# Source: https://fastapi.tiangolo.com/async/#in-a-hurry
# Generated by Whetstone — do not edit manually

import ast
import glob

def test_fastapi_route_handlers_are_async():
    """Route handlers decorated with FastAPI route decorators MUST use async def."""
    route_decorators = {'get', 'post', 'put', 'delete', 'patch', 'head', 'options'}
    violations = []

    for filepath in glob.glob('src/**/*.py', recursive=True):
        with open(filepath) as f:
            tree = ast.parse(f.read())

        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef) and not isinstance(node, ast.AsyncFunctionDef):
                for decorator in node.decorator_list:
                    dec_name = _get_decorator_name(decorator)
                    if dec_name and dec_name.split('.')[-1] in route_decorators:
                        violations.append(f"{filepath}:{node.lineno} - {node.name} is sync")

    assert not violations, f"Sync route handlers found:\n" + "\n".join(violations)
```

---

## Strictness: The "5 Rules You Trust" Philosophy

Whetstone is not a linter. It doesn't try to catch everything. It catches the things that matter most and that nothing else catches.

### Extraction Filters

The extraction prompt enforces these hard filters:

1. **Confidence threshold**: If you're not 90%+ confident this rule prevents a real mistake, don't propose it.
2. **Signal requirement**: Every rule MUST have at least one deterministic signal (`ast` or `pattern`). Pure-AI rules are rejected.
3. **Count ceiling**: Maximum 5 rules per dependency. If you can't rank them, you haven't filtered hard enough.
4. **Novelty requirement**: Don't propose rules that standard linters already enforce (ruff, biome, clippy defaults).
5. **Source backing**: Every rule must cite a specific URL in the dependency's documentation. No "general best practice" rules.

### What Gets Rejected

- "Write clean code" (too vague, no signal)
- "Use meaningful variable names" (linters + common sense)
- "Add error handling" (too broad)
- "Follow the single responsibility principle" (architecture, not testable)
- "Use TypeScript strict mode" (tsconfig setting, not a code pattern)
- Rules that duplicate ruff/biome/clippy defaults

### What Gets Accepted

- "FastAPI route handlers MUST use async def" (AST-checkable, docs-backed, commonly missed)
- "Pydantic v2: use `model_json_schema()` not `.schema()`" (migration footgun, pattern-checkable)
- "Next.js 15: use `async` for page components that access params" (breaking change, AST-checkable)
- "Tokio: use `#[tokio::main]` not `block_on` at top level" (convention divergence, AST-checkable)

### Pattern Detection Filters

`detect-patterns.py` applies its own strictness:

- **Frequency floor**: Pattern must appear in 3+ separate sessions or commits to be proposed
- **Recency bias**: Patterns from last 30 days weighted 3x vs older patterns
- **Dedup against existing**: If a pattern matches an existing rule or linter config, skip it
- **Actionability check**: Pattern must be expressible as a testable signal, not just a preference

---

## Workflow

### Full Init (First Run)

```
User: "Run whetstone init"

1. Agent reads SKILL.md, activates Whetstone workflow
2. Agent runs: python3 scripts/detect-deps.py
   → Shows: "Found 12 dependencies across Python + TypeScript"
   → Lists deps, user confirms which to extract rules for

3. Agent runs: python3 scripts/resolve-sources.py --deps <selected>
   → Shows: "Resolved docs for 8/12 deps, 3 have llms.txt"
   → Fetches content for each

4. Agent runs: python3 scripts/detect-patterns.py
   → Shows: "Found 4 recurring style patterns in your history"
   → Lists patterns with evidence

5. Agent reads source content + patterns
   → Applies extraction prompt (high-confidence-or-silence)
   → Proposes rules with signals and golden examples
   → Maximum 5 per dependency

6. Interactive review:
   → Agent presents each rule with source, signals, examples
   → User: approve / deny / edit
   → Approved rules saved to whetstone/rules/*.yaml

7. Agent runs: python3 scripts/generate-agent-context.py
   → Generates CLAUDE.md, AGENTS.md, .cursorrules (per config)

8. Agent runs: python3 scripts/generate-tests.py
   → Generates test files + linter configs

9. Agent shows summary:
   → "Generated 14 rules across 5 deps + 2 style patterns"
   → "Tests: whetstone/evals/{python,typescript,rust}/"
   → "Agent context: CLAUDE.md, AGENTS.md"
```

### Update (Subsequent Runs)

```
User: "Update whetstone rules"

1. Agent runs: python3 scripts/detect-deps.py --check-drift
   → Shows which deps changed version since last extract

2. Agent runs: python3 scripts/resolve-sources.py --changed-only
   → Re-fetches only changed source content

3. Agent runs: python3 scripts/detect-patterns.py --since-last-run
   → Shows new patterns since last run

4. Agent compares old vs new source content
   → Proposes new/modified/removed rules

5. Interactive review (only changes)
   → User approves diffs

6. Regenerate affected outputs
```

---

## File Structure

### The Skill Repo (what you publish / `npx skills add`)

```
whetstone/
├── SKILL.md                        # Agent skill instructions
├── scripts/
│   ├── detect-deps.py             # Dependency detection (Python/TS/Rust)
│   ├── resolve-sources.py         # Source URL resolution + fetching
│   ├── detect-patterns.py         # Mine transcripts/git/PRs for style patterns
│   ├── generate-agent-context.py  # Multi-format agent context generation
│   └── generate-tests.py          # Test + linter config generation
├── references/
│   ├── rule-schema.yaml           # Rule YAML format reference
│   ├── extraction-prompt.md       # The full extraction prompt
│   └── signal-strategies.md       # Signal types and decomposition guide
└── assets/
    └── whetstone.yaml.template    # Default config template
```

### Project Output (what Whetstone creates in user's repo)

```
whetstone/
├── whetstone.yaml                  # Config: languages, sources, trigger mode, agents
├── rules/
│   ├── python/
│   │   ├── fastapi.yaml
│   │   └── pydantic.yaml
│   ├── typescript/
│   │   └── next.yaml
│   ├── rust/
│   │   └── tokio.yaml
│   └── patterns/
│       └── style-conventions.yaml  # Rules from detect-patterns
├── evals/
│   ├── python/
│   │   ├── test_fastapi_async_routes.py
│   │   ├── test_pydantic_v1_validator.py
│   │   └── conftest.py
│   ├── typescript/
│   │   ├── next-page-async-params.test.ts
│   │   └── setup.ts
│   └── rust/
│       └── whetstone_tokio_tests.rs
├── lint/
│   ├── ruff.whetstone.toml
│   ├── biome.whetstone.json
│   └── clippy.whetstone.toml
└── .last-run                       # Timestamp for --since-last-run

# Agent context files (project root, committed)
CLAUDE.md
AGENTS.md
.cursorrules
.github/copilot-instructions.md
```

---

## Build Order

| Block | Est. Time | Deliverable | Description |
|-------|-----------|-------------|-------------|
| 1     | 1.5h      | `SKILL.md` | Core skill: workflow instructions, extraction prompt, rule format, strictness criteria, agent context templates. This is the most important file — it defines the entire experience. |
| 2     | 1h        | `detect-deps.py` | Parse pyproject.toml, package.json, Cargo.toml. Output structured JSON. Include `--check-drift` flag for update flow. |
| 3     | 1.5h      | `resolve-sources.py` | Query PyPI/npm/crates.io APIs, probe for llms.txt, fetch content, hash it. Output structured JSON with content. |
| 4     | 1.5h      | `detect-patterns.py` | Mine Claude transcripts (JSONL parsing), git log/diff analysis, optional GH PR comments. Pattern grouping, frequency scoring, dedup. |
| 5     | 1h        | `generate-agent-context.py` | Read rules YAML, generate CLAUDE.md + AGENTS.md + .cursorrules + copilot-instructions.md. Configurable format list. |
| 6     | 1.5h      | `generate-tests.py` | Generate pytest/vitest/cargo-test files from rule signals. Generate linter config overlays (ruff/biome/clippy). |
| 7     | 0.5h      | References + config | rule-schema.yaml, extraction-prompt.md, signal-strategies.md, whetstone.yaml.template. End-to-end smoke test. |

**Total: ~8.5 hours**

---

## What's in the MVP vs What's Deferred

### In the MVP (80% of value)

| Feature | How |
|---------|-----|
| Dependency detection (Python/TS/Rust) | `detect-deps.py` |
| Source resolution + llms.txt | `resolve-sources.py` |
| LLM-based rule extraction | Agent + SKILL.md extraction prompt |
| Style pattern detection | `detect-patterns.py` |
| Interactive approval | Native agent conversation |
| Rule YAML persistence | Agent writes files per skill instructions |
| Agent context generation (6 formats) | `generate-agent-context.py` |
| Test generation (AST/pattern) | `generate-tests.py` |
| Lint config generation | Ruff + Biome + Clippy overlays |
| Signal decomposition model | Embedded in extraction prompt |
| Golden examples | Part of rule format |
| Multiple trigger modes | Manual, session hook, post-merge, scheduled |

### Deferred to Post-MVP

| Feature | Why Deferred |
|---------|-------------|
| Layer system (personal/project/team) | Adds complexity, single project scope is fine for validation |
| Source monitoring/status command | Manual re-run is acceptable for MVP; scheduled mode partially covers this |
| AI eval runner + calibration | Tests are generated but threshold-gating is deferred |
| Team config + extends | Requires registry/package infrastructure |
| Promote command | Needs layer system first |
| Shared rule registry | Major infrastructure, needs community first |
| Signal promotion (AI → deterministic) | Requires historical eval data |
| Rust binary | Scripts validate the experience first; rewrite only if needed |

---

## Transition Path

1. **Validate extraction quality** — Does the skill prompt + agent produce good rules? Iterate on the extraction prompt in `references/extraction-prompt.md`.
2. **Validate the value** — Do generated tests catch real problems? Do agent context files improve agent output? Measure by running on real projects.
3. **If validated** → Build Rust CLI that internalizes what the scripts do, adds layer system, source monitoring, eval runner. The YAML rule format and file structure are identical, so all rules carry over.
4. **Keep the skill** — Even with a CLI, the skill remains the agent-facing interface. CLI handles deterministic work; skill teaches agents how to use the CLI.
