# Product Spec

## Overview

Whetstone is a CLI tool that derives coding rules from the documentation of your actual dependencies, decomposes them into the most deterministic checks possible, and generates three outputs from those rules: native tests and lint configurations to enforce them after code is written, and agent context files to inform AI coding agents before code is written. It keeps all three current as your dependencies evolve.

It is a codegen tool, not a runtime dependency. It produces pytest files, vitest tests, cargo tests, linter configs, and agent instruction files (AGENTS.md, CLAUDE.md, .cursorrules) that work with your existing toolchain. A teammate who never installs Whetstone still gets every rule enforced through standard CI, and every agent guided by current instructions.

---

## Problem

Three things are broken in how AI coding agents (and humans) write code today:

1. **Rules go stale.** Linter configs and coding conventions are written once at project setup. Dependencies evolve, deprecate APIs, and introduce better patterns. Nobody updates the rules to match. Agents keep writing code against outdated practices because that’s what they’re told to do.
2. **Semantic best practices are unenforced.** “Error messages should be actionable,” “prefer composition over inheritance,” “use dependency injection over singletons” — the industry agrees on these, but no tool checks them. They live in style guides that agents never read and developers forget.
3. **Agents aren’t told what they need to know.** Even when best practices are known, they don’t make it into the context files agents read before writing code. AGENTS.md and .cursorrules are written once by hand — if they’re written at all — and never updated to reflect the current state of the project’s dependencies.

Whetstone solves all three by treating documentation as a living source of truth and converting it into enforceable checks that stay current *and* agent instructions that keep agents informed before they write a single line.

---

## How It Works

### 1. Init

Detects the project’s languages and dependencies from manifest files (`pyproject.toml`, `package.json`, `Cargo.toml`). Resolves documentation URLs from package registry metadata. Probes for `llms.txt` / `llms-full.txt` at each URL — these are the preferred source because they’re structured, concise, and purpose-built for LLM consumption. Falls back to full documentation when `llms.txt` isn’t available.

User confirms which languages to cover, toggles sources on/off, and optionally adds custom sources (team style guides, blog posts, any public URL).

Writes a `whetstone.yaml` config file.

### 2. Extract

Crawls configured sources. Sends content to an LLM with a structured extraction prompt. The extraction is deliberately narrow — it does not ask “what are all the best practices for this library.” It asks: **“what does this documentation warn about that developers commonly get wrong, and that isn’t already caught by standard linting?”**

This filters for high-value rules:

- **Migration footguns** — deprecated APIs that still work but shouldn’t be used
- **Non-obvious defaults** — things that work but perform badly or are insecure unless configured correctly
- **Convention divergence** — patterns the docs recommend that contradict what most tutorials or LLMs default to
- **Breaking change preparation** — patterns that will fail in the next major version
- **Semantic practices** — conventions that require judgment to enforce (error message quality, naming, architecture)

**The core principle is high confidence or silence.** If a dependency’s documentation doesn’t clearly state a best practice, Whetstone doesn’t invent one. A project with 40 dependencies might get rules for 8 of them. That’s correct — those are the 8 that have something worth enforcing. No rules is always better than low-confidence rules. Users can manually add rules for anything Whetstone doesn’t extract.

This typically yields 3-8 rules per dependency, not 30. The goal is the smallest set of rules that prevents the most common mistakes.

Each proposed rule includes:

- A description using RFC 2119 severity keywords (MUST, SHOULD, MAY)
- The source URL it was derived from
- A **relevance category** (migration, default, convention, breaking-change, semantic)
- A decomposition into **signals** — deterministic checks that can verify the rule without AI:
    - `ast` — verifiable via syntax tree analysis
    - `pattern` — verifiable via text or regex matching
    - `lint_proxy` — maps directly to an existing linter rule
    - `ai` — requires LLM judgment (last resort)
- **Golden examples** — 3-5 known-pass and known-fail code samples for calibration
- A **confidence score** indicating how explicitly the source stated the rule vs. how much was inferred

Users can control extraction depth:

```bash
whetstone extract                   # Default: high-value rules only (3-8 per dep)
whetstone extract --depth full      # Everything the LLM can derive
whetstone extract --depth minimal   # Only MUST-level migration and breaking changes
```

User reviews each candidate interactively: approve, deny, edit, or skip. Only approved rules are persisted.

### 3. Generate

Reads the merged ruleset across all layers and produces three categories of output from the same approved rules:

**Post-generation enforcement:**
- **Native test files** for each rule’s deterministic signals — pytest (Python), vitest (TypeScript), cargo test (Rust). These use language-native AST tooling (`ast` stdlib, `ts-morph`, `syn`).
- **Lint config overlays** that map `lint_proxy` rules to the project’s linter — ruff (Python), biome (TypeScript), clippy (Rust).
- **AI eval definitions** for signals that can’t be checked deterministically. These are YAML files defining a targeted binary question, few-shot golden examples, and the AST-based pre-filter that selects which code to evaluate.
- **Calibration tests** that verify AI eval prompts agree with golden examples before running on real code.

**Pre-generation guidance:**
- **Agent context files** that tell AI coding agents what patterns to use, what to avoid, and why — before they write code. Generated in whichever formats the user configures: AGENTS.md, CLAUDE.md, .cursorrules, or all of them. The content isn’t a raw dump of rules — it’s structured instructions with reasoning from the source documentation, so the agent understands the *why* and can generalise to cases the rules don’t cover explicitly.

The user configures which agent formats to target in `whetstone.yaml`:

```yaml
agents:
- agents.md       # Universal (AGENTS.md)
- claude.md       # Claude Code (CLAUDE.md)
- cursorrules     # Cursor (.cursorrules)
```

All generated files are committed to the repo (except personal-layer outputs, which are gitignored). Every file includes a header comment linking it to the rule ID, source URL, and layer. When rules update, all three outputs — tests, lint configs, and agent context files — regenerate together.

### 4. Status

Checks all configured sources for changes. Reports specific findings and specific recommendations — not “your rules are stale” but “Pydantic v2.11 deprecated `schema()` — we recommend adding a rule to use `model_json_schema()` instead.” Compares source content hashes and dependency versions against what’s stored in the rule files.

### 5. Update

Re-crawls only the sources that have changed. Extracts rules from the **diff**, not the entire documentation. Presents each proposed new or modified rule for approval with full context: what changed in the source, what the rule would enforce, and how it would be tested. Regenerates only the affected test files, lint configs, and agent context files after approval.

A `--check` flag for CI exits non-zero if sources have drifted without rule updates — a freshness check, like a lockfile verification.

### 6. Promote

Moves a rule from one layer to another. A personal preference becomes a project standard, or a project rule becomes an org-wide policy. Regenerates tests and agent context files in the appropriate output location.

---

## The Eval Model

The core insight: semantic rules aren’t binary “deterministic or AI.” Every rule is a spectrum of signals, and the goal is to maximise the deterministic coverage before resorting to AI judgment.

### Signal Decomposition

Take “error messages SHOULD be actionable.” This decomposes into:

| Signal | Strategy | Deterministic? |
| --- | --- | --- |
| Uses dynamic string formatting (f-string, .format) | AST | Yes |
| References a variable from the surrounding scope | Pattern | Yes |
| Contains expectation language (“expected”, “got”, “must be”) | Pattern | Yes |
| Suggests a remediation or next step | AI | No |

Three of four signals are fully deterministic. The eval runs them first.

### Threshold Gating

Each rule defines pass and fail thresholds based on its deterministic signals:

- All deterministic signals present → **auto-pass**, no AI needed
- Zero deterministic signals present → **auto-fail**, no AI needed
- In between → **ambiguous**, send to AI for a narrow binary judgment

This means AI eval costs scale with ambiguity, not codebase size. A well-decomposed rule might send 20% of candidates to the LLM instead of 100%.

### Targeted AI Judgment

When AI is needed, it receives:

- A specific binary question (not “review this code”)
- The relevant code snippet with surrounding context
- 2-3 golden examples as few-shot grounding
- Instructions to answer PASS or FAIL with a one-line reason

### Calibration

Before running AI evals on real code, Whetstone runs them against the rule’s golden examples. If the LLM disagrees with known verdicts, the prompt needs fixing — not the code. This catches model drift, prompt regressions, and provider changes.

### Signal Promotion

Over time, patterns in AI judgments can be identified and promoted to new deterministic signals. If the AI judge consistently fails messages that don’t contain the word “expected” or a comparison operator, that becomes a new pattern signal. The AI dependency shrinks as the system learns.

---

## Layer Model

Rules cascade through three scopes, like git config or ESLint. More specific layers override broader ones.

### Layers

| Layer | Location | Committed | CI Visible | Purpose |
| --- | --- | --- | --- | --- |
| **Whetstone built-in** | Ships with the binary | — | Via generated tests | Curated language-level best practices |
| **Team / Org** | Standalone repo or package | In its own repo | Via `extends` | Org-wide standards |
| **Project** | `<repo>/whetstone/` | Yes | Yes | Project-specific rules from project deps |
| **Personal** | `<repo>/whetstone/.personal/` | No (gitignored) | No | Your preferences, your sources |

Resolution order: personal > project > team > built-in. A personal deny overrides a team MUST.

### Built-in Rules

Whetstone ships a curated set of language-level rules per supported language. These are not comprehensive style guides — they are the rules that are **most commonly missed or unenforced** by standard tooling: semantic conventions, architectural patterns, and practices that linters don’t cover.

The built-in set is intentionally small and high-signal. It’s maintained by the Whetstone project and refined over time based on community violation data and feedback. Users can override severity or deny any built-in rule.

Think of it as `extends: whetstone:recommended` — a strong, opinionated baseline that earns its place by catching real problems.

### Personal Layer Isolation

Personal-layer tests are generated into `<repo>/whetstone/.personal/`, which is automatically added to `.gitignore` by `whetstone init`. This means:

- `pytest whetstone/evals/python/` locally runs everything — project and personal tests together
- CI runs the same path but only sees committed files — personal evals are invisible
- No separate commands or paths to remember

Agent context files (AGENTS.md, CLAUDE.md, .cursorrules) are generated from **committed rules only** — project and team layers. Personal rules don’t leak into agent context files that get committed to the repo. If a user wants their personal rules to influence their local agent, they can generate a personal agent context file to `whetstone/.personal/` and configure their agent to read it locally.

### Team Layer

A team config is a standalone repository or package containing shared sources, rules, and settings. Projects reference it via `extends`:

```yaml
extends:
- whetstone:recommended
- my-org/whetstone-config
```

Multiple extends are supported. Later entries override earlier ones. Project rules override team rules. Personal rules override everything.

### Sharing (Future)

Individuals and teams can publish their rulesets as packages or public configs. Other users can extend them:

```yaml
extends:
- whetstone:recommended
- @someuser/fastapi-strict
- my-org/standards
```

A shared rule registry would cache pre-extracted rules for popular dependencies, reducing LLM cost for common setups and enabling discovery (“show me popular Whetstone rulesets for FastAPI projects”).

---

## Source Resolution

### Priority Order

1. `llms.txt` / `llms-full.txt` at the dependency’s docs URL — cheapest, fastest, purpose-built
2. Changelog and migration guide pages — highest signal for “what changed”
3. Full documentation crawl — most expensive, last resort

### Dependency Detection

| Language | Manifest | Registry | AST Tooling | Test Framework | Linter |
| --- | --- | --- | --- | --- | --- |
| Python | `pyproject.toml`, `requirements.txt` | PyPI | `ast` (stdlib) | pytest | ruff |
| TypeScript | `package.json` | npm | ts-morph | vitest | biome |
| Rust | `Cargo.toml` | crates.io / docs.rs | syn | cargo test | clippy |

### Custom Sources

Users can add any public URL as a source — blog posts, internal style guides, conference talks. These are crawled and processed identically to dependency docs.

---

## CLI Commands

```
whetstone init [--personal | --team]     Set up config at the appropriate layer
whetstone extract [--source <n>] [--depth default|full|minimal]
                                         Crawl sources, propose rules, approve interactively
whetstone generate [--lang <lang>]       Generate tests, lint configs, and agent context files
whetstone status                         Check source freshness, show specific recommendations
whetstone update [--check]               Re-extract changed sources, propose rule diffs
whetstone promote <rule-id> --to <layer> Move a rule between layers
whetstone check --ai-only [path]         Run AI eval definitions (the one runtime command)
whetstone check --calibrate              Verify AI evals against golden examples
```

### Running Generated Tests

Whetstone does not run deterministic tests. Standard tooling does:

```bash
pytest whetstone/evals/python/
npx vitest run whetstone/evals/typescript/
cargo test --test whetstone
ruff check --config whetstone/lint/ruff.whetstone.toml src/
```

---

## File Structure

### Project Level

```
whetstone/
├── whetstone.yaml              # Config: languages, sources, extends, agents, settings
├── rules/
│   ├── python/
│   │   ├── fastapi.yaml        # Rules extracted from FastAPI docs
│   │   ├── pydantic.yaml       # Rules extracted from Pydantic docs
│   │   └── ...
│   └── typescript/
│       ├── next.yaml
│       └── ...
├── evals/
│   ├── python/
│   │   ├── test_fastapi_async_routes.py
│   │   ├── test_pydantic_v1_validator.py
│   │   ├── conftest.py
│   │   └── ...
│   ├── typescript/
│   │   ├── next-page-export.test.ts
│   │   ├── setup.ts
│   │   └── ...
│   └── ai/
│       ├── actionable-error-messages.yaml
│       └── ...
├── lint/
│   ├── ruff.whetstone.toml
│   └── biome.whetstone.json
├── .personal/                  # Gitignored — personal layer
│   ├── rules/
│   ├── evals/
│   └── lint/
└── .cache/                     # Gitignored — source content cache

# Agent context files (written to project root, committed)
AGENTS.md                       # Universal agent instructions
CLAUDE.md                       # Claude Code instructions
.cursorrules                    # Cursor instructions
```

### Personal Level

```
~/.whetstone/
├── config.yaml                 # LLM provider, default languages, global prefs
└── sources.yaml                # Personal sources (applied to all projects)
```

### Rule File Format

```yaml
# whetstone/rules/python/fastapi.yaml

source:
name: fastapi
docs_url: https://fastapi.tiangolo.com
llms_txt: https://fastapi.tiangolo.com/llms.txt
version:"0.115.0"
content_hash: a3f2c8e1...

rules:
-id: fastapi.async-routes
severity: must
confidence: high
category: convention  # migration | default | convention | breaking-change | semantic
    description:>
      Route handlers MUST use async def. Sync handlers block the
      event loop and degrade performance under concurrent load.
source_url: https://fastapi.tiangolo.com/async/#in-a-hurry
approved:true
approved_at: 2025-02-08T12:00:00Z

signals:
-id: is-sync-function
strategy: ast
description: Function decorated with route decorator uses def instead of async def
weight: required

golden_examples:
      -code:|
          @app.get("/users")
          async def get_users():
              return await db.fetch_users()
verdict: pass

      -code:|
          @app.get("/users")
          def get_users():
              return db.fetch_users()
verdict: fail

-id: fastapi.actionable-errors
severity: should
confidence: medium
category: semantic  # migration | default | convention | breaking-change | semantic
    description:>
      Error responses SHOULD include the invalid value received
      and what was expected.
source_url: https://fastapi.tiangolo.com/tutorial/handling-errors/
approved:true
approved_at: 2025-02-08T12:00:00Z

signals:
-id: uses-dynamic-message
strategy: ast
description: HTTPException detail uses f-string or .format()
weight: required

-id: references-input-value
strategy: pattern
description: Message references a variable from the surrounding scope
weight: strong

-id: contains-expectation
strategy: pattern
description: Message contains expectation language
weight: moderate

-id: suggests-fix
strategy: ai
description: Message suggests how to resolve the issue
weight: moderate

deterministic_pass_threshold:3
deterministic_fail_threshold:0

ai_eval:
trigger: ambiguous
      question:|
        This error message uses dynamic values but may not clearly
        communicate what was expected. Does it help the developer
        understand what went wrong and what to do about it?
        Answer PASS or FAIL with a one-line reason.
context_lines:10

golden_examples:
      -code:|
          raise HTTPException(
              status_code=422,
              detail=f"User age must be positive, got {age}"
          )
verdict: pass
reason: States constraint, includes actual value

      -code:|
          raise HTTPException(status_code=400, detail="Invalid input")
verdict: fail
reason: Generic, no value, no expectation

      -code:|
          raise HTTPException(
              status_code=404,
              detail=f"User {user_id} not found"
          )
verdict: pass
reason: Identifies the entity and the value looked up

      -code:|
          raise HTTPException(status_code=500, detail="Something went wrong")
verdict: fail
reason: Completely generic, no actionable information
```

---

## Implementation

### Language

> **Current (MVP):** Python scripts shipped as an Agent Skill. Requires Python 3.9+ and PyYAML. Any skills-compatible agent acts as the LLM — no separate API key or binary needed.
>
> **Planned (future phase):** Rust CLI binary. The YAML rule format and file structure carry over unchanged.

Rationale for future Rust target:
- Fast startup for a CLI that developers run frequently
- Single binary distribution — no Python/Node runtime required on the user's machine
- Cross-platform (Linux, macOS, Windows) via cross-compilation
- Can use `tree-sitter` bindings natively for AST analysis across all three target languages
- Rust ecosystem has strong libraries for YAML (serde), HTTP (reqwest), and CLI (clap)

The tool generates Python, TypeScript, and Rust test files. The current implementation is Python; the planned Rust CLI will internalize the same logic.

### Architecture

```
src/
├── main.rs
├── cli/                            # Command definitions (clap)
│   ├── init.rs
│   ├── extract.rs
│   ├── generate.rs
│   ├── status.rs
│   ├── update.rs
│   ├── promote.rs
│   └── check.rs
├── config/
│   ├── loader.rs                   # Resolves and merges all layers
│   ├── personal.rs                 # ~/.whetstone/ management
│   ├── project.rs                  # <repo>/whetstone/ management
│   └── team.rs                     # Team config resolution (extends)
├── sources/
│   ├── detector.rs                 # Reads pyproject.toml, package.json, Cargo.toml
│   ├── resolver.rs                 # Dep name → docs URL → llms.txt URL
│   ├── crawler.rs                  # Fetches source content (llms.txt priority)
│   └── differ.rs                   # Hash-based change detection
├── extraction/
│   ├── extractor.rs                # LLM-based rule derivation
│   ├── diff_extractor.rs           # Diff-only extraction for updates
│   └── prompts/                    # Prompt templates (embedded or files)
│       ├── extract_rules.md
│       ├── extract_diff.md
│       └── classify_signals.md
├── codegen/
│   ├── python/
│   │   ├── ast_tests.rs            # Generates pytest files
│   │   ├── pattern_tests.rs        # Generates pytest files with regex
│   │   └── ruff_config.rs          # Generates ruff overlay
│   ├── typescript/
│   │   ├── ast_tests.rs            # Generates vitest files
│   │   ├── pattern_tests.rs        # Generates vitest files with regex
│   │   └── biome_config.rs         # Generates biome overlay
│   ├── rust/
│   │   ├── ast_tests.rs            # Generates cargo test files
│   │   └── clippy_config.rs        # Generates clippy overlay
│   ├── ai_evals.rs                 # Generates AI eval YAML definitions
│   ├── agent_context/
│   │   ├── mod.rs                  # Orchestrates agent context generation
│   │   ├── agents_md.rs            # AGENTS.md formatter
│   │   ├── claude_md.rs            # CLAUDE.md formatter
│   │   └── cursorrules.rs          # .cursorrules formatter
│   └── templates/                  # Embedded templates (include_str!)
│       ├── python_ast_test.py.tera
│       ├── python_conftest.py.tera
│       ├── typescript_ast_test.ts.tera
│       ├── typescript_setup.ts.tera
│       ├── rust_ast_test.rs.tera
│       ├── ai_eval.yaml.tera
│       ├── agents_md.tera
│       ├── claude_md.tera
│       └── cursorrules.tera
├── runners/
│   ├── ai_judge.rs                 # Runs AI eval definitions
│   └── calibration.rs              # Validates AI evals against golden examples
├── layers/
│   ├── merge.rs                    # Layer resolution logic
│   └── promote.rs                  # Cross-layer rule movement
└── core/
    ├── rules.rs                    # Rule model + serde serialisation
    ├── signals.rs                  # Signal model + threshold logic
    ├── models.rs                   # Shared types (Violation, CheckResult)
    └── llm.rs                      # LLM client abstraction (Anthropic, OpenAI, etc.)
```

### Key Dependencies (Rust)

| Crate | Purpose |
| --- | --- |
| `clap` | CLI framework |
| `serde` + `serde_yaml` | YAML serialisation |
| `reqwest` | HTTP client for source crawling and LLM API calls |
| `tokio` | Async runtime |
| `tree-sitter` + language grammars | AST parsing for Python, TypeScript, Rust |
| `tera` | Template engine for test file generation |
| `indicatif` | Progress bars and spinners |
| `dialoguer` | Interactive prompts (approve/deny/edit) |
| `console` | Terminal styling |
| `sha2` | Content hashing for change detection |

### LLM Client

The `llm` module abstracts over LLM providers. MVP supports Anthropic (Claude). The interface is minimal:

```rust
pub trait LlmClient {
    async fn complete(&self, prompt: &Prompt) -> Result<String>;
}
```

Used in exactly two places:
1. **Extraction** — deriving rules from source content (runs during `extract` and `update`)
2. **AI evaluation** — judging ambiguous cases (runs during `check --ai-only`)

Everything else is deterministic.

---

## Distribution

### Current (MVP) — Agent Skill

```bash
# Install as an agent skill
npx skills add whetstone

# Or clone directly
git clone https://github.com/yourusername/whetstone.git
pip install pyyaml
```

### CI Usage (Current)

Uses the GitHub Action composite wrapper:

```yaml
- uses: whetstone/whetstone@main
  with:
    fail-on: stale
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

### Planned — Binary Releases (Future Phase)

> The following is planned for the Rust CLI phase, not currently shipped.

Pre-built binaries for Linux (x86_64, aarch64), macOS (x86_64, aarch64), and Windows (x86_64) via GitHub Releases. Install via `brew install whetstone`, `cargo install whetstone`, or `curl -fsSL https://whetstone.dev/install.sh | sh`.

But note: Whetstone is only needed in CI for `update --check` (freshness) and `check --ai-only` (AI evals). The generated tests and lint configs run without Whetstone installed.

---

## Build Order

### Slice 1: Foundation + Python Extraction

- Project structure, CLI skeleton (clap), config types (serde)
- Dependency detection from `pyproject.toml` / `requirements.txt`
- Source URL resolution (PyPI metadata → docs URL → `llms.txt` probe)
- Source crawler (`llms.txt` path)
- LLM client (Anthropic)
- Extraction prompt with signal decomposition and golden examples
- Interactive approval UI (dialoguer)
- Rule YAML serialisation
- `whetstone init` + `whetstone extract` working for Python

### Slice 2: Python Codegen

- Template engine setup (tera)
- AST test generation for Python (pytest + ast module)
- Pattern test generation for Python (pytest + regex)
- Ruff config overlay generation
- AI eval definition generation
- conftest.py generation
- `whetstone generate` working for Python

### Slice 3: Layer System

- Personal config (`~/.whetstone/`)
- Project-level `.personal/` directory with gitignore management
- Layer merge logic (personal > project > built-in)
- `whetstone init --personal`
- Output routing (project evals → committed, personal evals → gitignored)
- `whetstone promote`

### Slice 4: Evals + Calibration

- AI eval runner (`check --ai-only`)
- Signal threshold logic (auto-pass, auto-fail, ambiguous routing)
- Calibration runner (`check --calibrate`)
- Golden example validation

### Slice 5: Source Monitoring

- Content hash storage and comparison
- Dep version drift detection
- Diff extraction (changed sources only)
- `whetstone status` with specific recommendations
- `whetstone update` with interactive approval
- `whetstone update --check` for CI

### Slice 6: TypeScript Support

- `package.json` detection + npm registry resolution
- tree-sitter TypeScript grammar integration
- vitest test generation via tera templates
- biome config overlay generation
- Full cycle across all commands

### Slice 7: Team Layer

- Team config repo structure and resolution
- `extends` parsing (git repos, local paths)
- Three-layer merge
- `whetstone init --team`

### Slice 8: Rust Support

- `Cargo.toml` detection + crates.io / docs.rs resolution
- tree-sitter Rust grammar integration
- cargo test generation via tera templates
- clippy config overlay generation

---

## Roadmap

### Shared Rule Registry

Pre-extracted rules for popular dependencies, ranked by community signal. Not just a cache — a curated corpus where the most valuable rules rise to the top.

**What gets stored:**
- Extracted rules for popular dependencies across all supported languages
- Anonymised usage metrics: approval rate, violation frequency, retention, false positive rate

**Ranking signals (rule goes up):**
- High approval rate — most users who see this rule keep it
- High violation frequency — this rule catches real problems in real codebases
- High retention — users don’t disable it after a week
- Explicit upvotes — users can flag a rule as particularly valuable
- Cross-project consistency — the rule is approved across many different types of project

**Ranking signals (rule goes down):**
- High deny rate — most users reject this rule during review
- High false positive rate — users disable it after generating tests
- Redundancy — already enforced by default linter configs
- Low violation rate — never actually triggers, so adds noise without value

**How it integrates:**
- When you add a dependency as a source, Whetstone checks the registry first
- If community-vetted rules exist, they’re presented ranked by composite score instead of running a fresh extraction
- The default shows the top rules (typically 3-8 per dependency). `--depth full` shows the full ranked list.
- Fresh extraction still runs if no registry data exists or if you want rules from a source the registry doesn’t cover
- Your approval/denial/violation data feeds back into the registry (anonymised, opt-in)
- Local approval is always required — the registry suggests, you decide

**Individual and team publishing:**
- Users and teams can publish their rulesets to the registry
- Published rules are tagged with the publisher and any overrides they’ve applied
- Others can extend them: `extends: @someuser/fastapi-strict`
- Voting applies to published rulesets too — popular community configs surface in search
- Think npm packages but for coding convention rulesets

### Advanced Agent Integration

Agent context generation (AGENTS.md, CLAUDE.md, .cursorrules) is part of core `generate`. Future work includes: discovering and recommending community-maintained skills and MCP servers relevant to the user’s dependencies, automatically suggesting MCP configurations during `init`, and flagging when community skills are stale relative to the user’s dependency versions.

### Signal Promotion

System observes patterns in AI judge verdicts and proposes new deterministic signals. AI dependency shrinks over time as more patterns are codified into AST and pattern checks.

### Rule Evolution

Track violations over time. Surface which rules are violated most. Propose clearer rewrites or stronger examples for rules that agents consistently get wrong. `whetstone evolve` command.

### Whetstone as a Service

GitHub App that monitors repos, runs extraction with pooled LLM access, draws from the shared rule registry, and surfaces recommendations. The registry’s ranking data improves with every user — more adoption means better signal on which rules matter. Free for public repos, paid for private. The Dependabot/Renovate model applied to coding conventions.