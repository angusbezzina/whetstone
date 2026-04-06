# Roadmap

## Phase 1: Foundation (Weeks 1-3)

### Slice 1.1: Core Infrastructure

- Project scaffolding: Rust workspace, CLI skeleton (clap), config types (serde)
- YAML rule schema definition and serialisation
- LLM client abstraction (Anthropic API, structured output parsing)
- Source crawler (HTTP client, `llms.txt` fetching, content hashing)

### Slice 1.2: Python Extraction

- Dependency detection from `pyproject.toml` / `requirements.txt`
- Source URL resolution via PyPI metadata → docs URL → `llms.txt` probe
- Extraction prompt: high-confidence-or-silence, signal decomposition, golden examples
- Interactive approval UI (dialoguer + console)
- Rule persistence to YAML
- `whetstone init` + `whetstone extract` working end-to-end for Python

### Slice 1.3: Python Codegen

- Template engine setup (tera)
- AST test generation → pytest files using Python’s `ast` module
- Pattern test generation → pytest files using regex
- Ruff config overlay generation
- AI eval definition generation (YAML output)
- conftest.py generation with configurable source paths
- `whetstone generate` working for Python

### Slice 1.4: Agent Context Generation

- Agent format configuration in `whetstone.yaml` (`agents: [agents.md, claude.md, cursorrules]`)
- Template-based generation: rules → structured markdown instructions with reasoning from source docs
- AGENTS.md, CLAUDE.md, .cursorrules formatters (each agent format has its own conventions)
- Agent context includes: patterns to use (with why), patterns to avoid (deprecated APIs, footguns), conventions
- Generated from committed rules only (project + team layers, not personal)
- Regenerates alongside tests when rules change

**Milestone: Usable on real Python projects. Full init → extract → generate cycle. Generated tests run with pytest and ruff. Agent context files keep AI coding agents informed before they write code.**

---

## Phase 2: Eval System (Weeks 4-5)

### Slice 2.1: Signal Decomposition Runtime

- Threshold gating logic (auto-pass, auto-fail, ambiguous routing)
- Deterministic signal runner (executes AST and pattern checks before AI)
- Cost estimation for AI eval runs

### Slice 2.2: AI Eval Runner

- `whetstone check --ai-only` command
- Targeted binary questions with few-shot golden examples
- Structured output parsing (PASS/FAIL + reason)

### Slice 2.3: Calibration

- `whetstone check --calibrate` command
- Runs AI evals against golden examples before real code
- Reports agreement rate, flags drift

**Milestone: Full eval pipeline working. Deterministic checks catch the majority, AI fills the gap on ambiguous cases, calibration ensures AI reliability.**

---

## Phase 3: Layer System (Weeks 6-7)

### Slice 3.1: Personal Layer

- `~/.whetstone/` config and sources management
- `whetstone init --personal`
- Personal source extraction and rule storage

### Slice 3.2: Project + Personal Merge

- Layer resolution logic (personal > project > built-in)
- `.personal/` directory in project with automatic `.gitignore` management
- Output routing: project rules → committed, personal rules → gitignored
- Single test command runs both layers locally, CI only sees committed

### Slice 3.3: Promote

- `whetstone promote <rule-id> --to project` command
- Moves rule YAML between layers
- Regenerates tests in the target layer’s output location

**Milestone: Developers can have personal preferences that coexist with project standards. Same `pytest` command locally and in CI, different visibility.**

---

## Phase 4: Source Monitoring (Weeks 8-9)

### Slice 4.1: Drift Detection

- Content hash comparison against stored hashes in rule files
- Dependency version drift detection (manifest vs whetstone.yaml)
- `whetstone status` with specific findings and specific recommendations

### Slice 4.2: Targeted Update

- Diff extraction: re-crawl only changed sources, extract rules from diff only
- Present specific new/modified/removed rules for approval
- Selective artifact regeneration (only affected tests and configs)
- `whetstone update` interactive flow

### Slice 4.3: CI Freshness Check

- `whetstone update --check` exits non-zero if sources have drifted
- Lightweight enough to run in CI on every push

**Milestone: The system nudges you when things change, tells you exactly what changed and what to do about it. No manual monitoring required.**

---

## Phase 5: TypeScript Support (Weeks 10-12)

### Slice 5.1: TypeScript Detection + Extraction

- `package.json` / lockfile dependency parsing
- npm registry metadata → docs URL → `llms.txt` resolution
- Extraction working for TypeScript dependencies

### Slice 5.2: TypeScript Codegen

- tree-sitter TypeScript grammar integration
- vitest test generation via tera templates
- biome config overlay generation
- setup.ts generation with shared test utilities

### Slice 5.3: TypeScript End-to-End

- Full init → extract → generate → status → update cycle for TypeScript
- Mixed Python + TypeScript projects working correctly

**Milestone: Full TypeScript support across all commands. Mixed-language projects work seamlessly.**

---

## Phase 6: Team Layer (Weeks 13-14)

### Slice 6.1: Team Config Structure

- Team config repo format (whetstone.yaml + sources.yaml + rules/)
- `whetstone init --team`

### Slice 6.2: Extends Resolution

- `extends` field parsing: git repos, local paths, published packages
- Multiple extends with ordered merge
- Three-layer resolution: personal > project > team > built-in

### Slice 6.3: Team Workflows

- Teams publish config repos, projects reference them
- Rule changes in team config propagate to all projects on next `update`

**Milestone: Organisations can define shared standards. Projects inherit and override. Personal preferences sit on top.**

---

## Phase 7: Rust Support (Weeks 15-17)

### Slice 7.1: Rust Detection + Extraction

- `Cargo.toml` dependency parsing
- crates.io / docs.rs metadata → docs URL → `llms.txt` resolution
- Extraction working for Rust dependencies

### Slice 7.2: Rust Codegen

- tree-sitter Rust grammar integration
- `cargo test` file generation via tera templates
- clippy config overlay generation

### Slice 7.3: Rust End-to-End

- Full cycle across all commands for Rust
- Mixed Python + TypeScript + Rust projects

**Milestone: All three target languages fully supported.**

---

## Phase 8: Built-in Rules (Weeks 18-19)

### Slice 8.1: Curated Rule Sets

- `whetstone:recommended` for Python, TypeScript, and Rust
- Language-level rules (not dependency-specific) focused on commonly missed semantic practices
- Versioned with Whetstone releases

### Slice 8.2: Built-in Layer Integration

- Built-in rules as the base layer in the cascade
- Users can override severity or deny any built-in rule
- Generated tests for built-in rules included by default

**Milestone: Whetstone is valuable out of the box before any sources are configured.**

---

## Phase 9: Shared Registry (Future)

### 9.1: Registry Infrastructure

- Central storage for pre-extracted rules per dependency
- API for publishing and fetching rule sets

### 9.2: Community Ranking

- Track anonymised usage metrics: approval rate, violation frequency, retention, false positive rate
- Composite score ranking: high-value rules surface, noise sinks
- Explicit upvote/downvote on rules

### 9.3: Publishing

- Users and teams publish rulesets to the registry
- `extends: @someuser/fastapi-strict` works as a package reference
- Discovery: search and browse popular rulesets by language, framework, style

### 9.4: Registry-First Extraction

- `whetstone extract` checks registry before running LLM extraction
- Community-vetted rules presented ranked by score
- Fresh extraction only for sources not in the registry
- User approvals feed back into registry rankings (opt-in)

**Milestone: Adding a popular dependency gives you high-quality rules instantly, for free, ranked by what the community has validated.**

---

## Phase 10: Advanced Agent Integration (Future)

### 10.1: Community Skill Discovery

- Index of community-maintained skills and MCP servers mapped to dependencies
- `whetstone init` recommends relevant community skills based on detected deps
- Version-aware matching: flag when a community skill is stale vs the user’s dep version
- `whetstone status` reports community skill updates alongside source changes

### 10.2: MCP Recommendations

- Discover and recommend MCP servers relevant to the user’s stack
- Suggest MCP configurations during `init` (e.g., “you’re using Postgres — here’s the community MCP”)
- Don’t build MCPs — just surface what exists and help configure

### 10.3: Signal Promotion

- System analyses patterns in AI judge verdicts over time
- Proposes new deterministic signals to replace recurring AI checks
- AI dependency shrinks as patterns are codified

### 10.4: Rule Evolution

- Violation tracking over time
- Surface which rules are violated most and by which agents
- Propose clearer rule descriptions or stronger golden examples
- `whetstone evolve` command

---

## Phase 11: Whetstone as a Service (Future)

### 11.1: GitHub App

- Monitors repos for dependency changes
- Runs extraction with pooled LLM access
- Surfaces recommendations via Issues with specific rule proposals
- User approves/denies, App generates PR with tests and configs

### 11.2: Business Model

- Free for public repos
- Paid for private repos
- Registry access included
- Team features (centralised dashboards, usage analytics, org-wide policies)