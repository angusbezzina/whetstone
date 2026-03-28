---
name: whetstone
description: >-
  Derives coding rules from dependency documentation and developer patterns,
  generates native tests, lint configs, and agent context files. Use when the
  user asks to extract rules, update standards, or run whetstone commands.
license: MIT
compatibility: Requires python3 (3.10+), git, and internet access for registry lookups.
metadata:
  author: whetstone
  version: "0.1.0"
---

# Whetstone

> Sharpen the tools that write your code.

Whetstone derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files.

## Activation

Activate when the user says any of: "whetstone", "whetstone doctor", "extract rules", "update standards", "update rules", "init whetstone", "run whetstone", "check rules", "refresh rules", "generate tests from rules".

If the user says "whetstone doctor", "doctor", "scan my project", or "bootstrap rules", use the **Doctor** workflow below — it's the fastest path from zero to working rules.

## Happy Path (TL;DR)

Most users need three steps:

1. **Bootstrap**: `python3 scripts/doctor.py --project-dir .` — detects deps, resolves docs, mines patterns
2. **Extract + Approve**: Read the doctor's `extraction_context`, apply the Extraction Prompt below for each source, present rules for approval using the Rule Card format
3. **Generate**: `python3 scripts/generate-tests.py --project-dir .` and `python3 scripts/generate-agent-context.py --project-dir .`

After that, check health with `python3 scripts/status.py --project-dir .` anytime.  
When deps update, run `python3 scripts/detect-deps.py --project-dir . --changed-only` to see what drifted, then re-extract only the changed ones.

See the **Doctor** workflow below for the detailed version.

---

## Script Paths

All scripts live in the `scripts/` directory relative to this SKILL.md file. When running scripts, use the absolute path based on where this skill is installed. For example, if this SKILL.md is at `/path/to/whetstone/SKILL.md`, run:

```
python3 /path/to/whetstone/scripts/detect-deps.py --project-dir .
```

In the workflow steps below, script paths are written as `scripts/detect-deps.py` — always resolve these relative to this SKILL.md's directory.

## Quick Reference

| Script | Purpose | Input | Output |
|--------|---------|-------|--------|
| `scripts/doctor.py` | **One-command bootstrap** | Project dir | JSON: extraction context |
| `scripts/status.py` | **Project health summary** | Rule YAML files | JSON: health dimensions |
| `scripts/ci-check.py` | **CI freshness check** | Project dir | JSON: CI outputs |
| `scripts/detect-deps.py` | Detect dependencies | Manifest files | JSON: deps list |
| `scripts/resolve-sources.py` | Resolve docs URLs | JSON from detect-deps | JSON: source content |
| `scripts/detect-patterns.py` | Mine style patterns | Transcripts, git, PRs | JSON: candidate patterns (project-scoped by default) |
| `scripts/generate-agent-context.py` | Generate agent files | Rule YAML files | AGENTS.md, CLAUDE.md, etc. |
| `scripts/generate-tests.py` | Generate tests + lint | Rule YAML files | pytest/vitest/cargo tests |

### Common Flags

All scripts accept `--project-dir` (default: `.`). User-facing scripts support these output modes:

| Flag | Behavior | Available in |
|------|----------|-------------|
| `--json` | JSON only to stdout (suppress human output) | doctor, status, ci-check |
| `--score` | Just the numeric score + label | status |
| `--pr-comment` | GitHub PR comment markdown | ci-check |
| `--changed-only` | Only process deps with drift | detect-deps, resolve-sources, ci-check |
| `--dry-run` | Preview without writing files | generate-agent-context, generate-tests |
| `--check-drift` | Include drift info in output | detect-deps |

Building-block scripts (detect-deps, resolve-sources, detect-patterns, generate-*) always output JSON to stdout. All scripts include a `next_command` field in their JSON output.

### JSON Output Contract

Every script follows this contract:

| Field | Type | Present | Description |
|-------|------|---------|-------------|
| `status` | `"ok"` \| `"error"` | Sometimes | Overall result status |
| `error` | string | On error | Human-readable error message |
| `next_command` | string | Always | Suggested next command to run |
| `warnings` | string[] | Sometimes | Non-fatal issues encountered |

**Success responses** include domain-specific data (e.g., `dependencies`, `sources`, `generated`).
**Error responses** always include `error` and `next_command` — never a traceback to stdout.

Progress messages go to stderr so stdout stays clean for JSON piping.

### Script vs Agent Responsibilities

| Task | Handled by | Why |
|------|-----------|-----|
| Dependency detection | **Script** | Deterministic manifest parsing |
| Source resolution | **Script** | Deterministic registry API calls |
| Pattern detection | **Script** | Deterministic transcript/git mining |
| Rule extraction | **Agent** | Requires reading and understanding documentation |
| Rule approval | **Agent + User** | Requires judgment and user consent |
| Test generation | **Script** | Deterministic code generation from approved YAML |
| Agent context generation | **Script** | Deterministic markdown generation from approved YAML |
| Health monitoring | **Script** | Deterministic metric computation |
| CI gating | **Script** | Deterministic pass/fail decision |

Scripts are interchangeable — they work regardless of which agent runs them. The agent brings judgment to steps 3 and 4; scripts handle everything else.

---

## Workflows

### Doctor (Recommended First Run)

Run when the user says "whetstone doctor", "doctor", "scan my project", "bootstrap rules", or when they want the fastest path from zero to working rules. This is the **recommended** entry point — it chains detect → resolve → patterns → extract → generate in one flow.

**Step 1: Run the doctor orchestrator**

```bash
python3 scripts/doctor.py --project-dir .
```

This runs dependency detection, source resolution, and pattern detection automatically. Progress is printed to stderr; the JSON result (including fetched source content) is printed to stdout.

Review the summary output. It will show:
- Dependencies found (runtime + dev, per language)
- Sources resolved (how many deps have docs, how many have llms.txt)
- Patterns found (recurring style signals from history)
- Warnings (any deps whose docs couldn't be resolved)

**Step 2: Extract rules from sources**

Read the `extraction_context` from the doctor output. For each source in `extraction_context.sources`, apply the **Extraction Prompt** below. Also consider any patterns from `extraction_context.patterns` as additional rule candidates.

Propose rules following the rule YAML schema. Maximum 5 rules per dependency. **Prioritize rules about recent changes (last 18 months) that LLMs were likely not trained on.**

**Step 3: Interactive approval**

Present proposed rules for approval. For each rule, show:
- **Rule ID** and **Description** (with RFC 2119 severity)
- **Category** and **Confidence**
- **Source URL** with the relevant quote from documentation
- **Signals** — how it will be checked (ast/pattern/lint_proxy/ai)
- **Why this matters** — what goes wrong if this rule is ignored
- **Why linters miss it** — brief note on why ruff/biome/clippy don't catch this
- **Golden examples** — pass and fail code

Batch option: offer to "approve all high-confidence rules" first, then review medium-confidence individually.

For each rule, the user can: **Approve**, **Edit**, **Deny**, or **Skip**.

Save approved rules to `whetstone/rules/{language}/{dependency}.yaml`.

**Step 4: Generate outputs**

```bash
python3 scripts/generate-agent-context.py --project-dir .
python3 scripts/generate-tests.py --project-dir .
```

**Step 5: Create config (if first run)**

If `whetstone/whetstone.yaml` doesn't exist, create it from `assets/whetstone.yaml.template` with the detected languages, confirmed agents list, and trigger mode (default: manual).

**Step 6: Summary**

Present a final summary:
- Rules: N approved across M dependencies + K style patterns
- Tests: paths to generated test files
- Agent context: which files were generated

**Next:** "Run your tests to verify: `pytest whetstone/evals/python/`" (or `npx vitest` / `cargo test` for the relevant language). Then run `python3 scripts/status.py --project-dir .` to confirm health.

---

### Init (First Run — Step-by-Step)

Run when the user says "whetstone init", "extract rules", or wants more control than the Doctor workflow provides. The Doctor workflow above is recommended for most users — use Init when you need to customize each step.

**Step 1: Detect dependencies**

```bash
python3 scripts/detect-deps.py --project-dir .
```

Present the findings: "Found N dependencies across [languages]." List the dependencies with name, version, and language. Ask the user which dependencies to extract rules for. Default: all non-dev dependencies.

**Step 2: Resolve documentation sources**

```bash
python3 scripts/detect-deps.py --project-dir . | python3 scripts/resolve-sources.py --deps dep1,dep2,dep3
```

Pass only the user-confirmed dependencies. Present: "Resolved docs for N/M deps, K have llms.txt." For any deps where resolution failed, note why and ask if the user wants to provide a manual docs URL.

**Step 3: Detect style patterns (optional)**

```bash
python3 scripts/detect-patterns.py --project-dir .
```

Present any discovered patterns with evidence (occurrence count, example quotes). Ask the user which patterns to include as rule candidates.

**Step 4: Extract rules**

Read the source content from Step 2 and patterns from Step 3. For each dependency, apply the extraction prompt below. When filling in the prompt template, use the `latest_version` and `latest_release_date` fields from the resolve-sources output. Set `{today}` to the current date. Propose rules following the rule YAML schema. Maximum 5 rules per dependency. **Prioritize rules about recent changes (last 18 months) that LLMs were likely not trained on.**

**Step 5: Interactive approval**

Present each proposed rule to the user with:
- Rule ID and description (using RFC 2119 severity: MUST, SHOULD, MAY)
- Category and confidence level
- Source URL
- Signals (how it will be checked)
- Golden examples (pass and fail code)

For each rule, the user can:
- **Approve** — save as-is
- **Edit** — modify description, severity, signals, or examples, then save
- **Deny** — discard (optionally note why for future reference)
- **Skip** — defer decision to later

Save approved rules to `whetstone/rules/{language}/{dependency}.yaml`.

**Step 6: Generate outputs**

```bash
python3 scripts/generate-agent-context.py --project-dir .
python3 scripts/generate-tests.py --project-dir .
```

Present summary: "Generated N rules across M deps. Tests: whetstone/evals/. Agent context: CLAUDE.md, AGENTS.md."

**Step 7: Create config (if first run)**

If `whetstone/whetstone.yaml` doesn't exist, create it from `assets/whetstone.yaml.template` with the detected languages, confirmed agents list, and trigger mode (default: manual).

**Next:** "Run your tests to verify: `pytest whetstone/evals/python/`" (or the equivalent for your language). Then run `python3 scripts/status.py --project-dir .` to confirm health.

### Update (Subsequent Runs)

Run when the user says "update whetstone", "refresh rules", "check for rule updates".

By default, update only processes dependencies that have changed (diff-only mode). Use this unless the user explicitly requests a full re-extraction.

**Step 1: Check for drift (changed deps only)**

```bash
python3 scripts/detect-deps.py --project-dir . --changed-only
```

This outputs only dependencies whose versions have drifted since last extraction. If no drift is found, inform the user and suggest running `whetstone status` instead. For a full check, use `--check-drift` (shows drift info but still outputs all deps).

**Step 2: Re-resolve changed sources only**

```bash
python3 scripts/detect-deps.py --project-dir . --changed-only | python3 scripts/resolve-sources.py --changed-only --project-dir .
```

Only re-fetches documentation for dependencies with version drift AND content changes. This is fast and avoids unnecessary network calls.

**Step 3: Check for new patterns**

```bash
python3 scripts/detect-patterns.py --project-dir . --since-last-run
```

Show any new patterns discovered since the last run.

**Step 4: Extract and diff**

For changed dependencies, re-run extraction. Compare proposed rules against existing rules. Present only the changes: new rules, modified rules, rules to remove.

**Step 5: Approve changes, regenerate**

Same approval flow as init, but only for changes. After approval, regenerate:

```bash
python3 scripts/generate-agent-context.py --project-dir .
python3 scripts/generate-tests.py --project-dir .
```

**Next:** "Run updated tests to verify: `pytest whetstone/evals/python/`". Then run `python3 scripts/status.py --project-dir .` to confirm the drift is resolved.

### Status

Run when the user says "whetstone status", "check health", "how are my rules", or similar.

```bash
python3 scripts/status.py --project-dir .
```

This outputs a compact health summary with five dimensions:
- **Freshness** — days since last rule extraction
- **Rules count** — total approved rules
- **High confidence ratio** — % of rules backed directly by documentation
- **Deterministic coverage** — % of signals that don't need AI (ast/pattern/lint_proxy)
- **Pending updates** — deps with version drift since last extraction

The output includes a status label (**Healthy**, **Needs Review**, **Stale**, **No Rules**) and specific recommendations with exact commands to run.

Present the human-readable summary to the user. If they want detail, offer `--json` for the full breakdown or `--score` for just the numeric score.

**Next:** Follow the `next_command` from the status JSON output — it suggests the most relevant action (e.g., `whetstone doctor` to re-resolve sources when drift is detected, or `whetstone doctor` for initial setup).

### Generate Only

Run when the user says "regenerate tests", "regenerate agent context", or when rules have been manually edited.

```bash
python3 scripts/generate-agent-context.py --project-dir .
python3 scripts/generate-tests.py --project-dir .
```

**Next:** "Run tests to verify the regenerated outputs." Then run `python3 scripts/status.py --project-dir .` to check overall health.

---

## Extraction Prompt

When extracting rules from dependency documentation, follow these instructions exactly.

### Your Task

You are reading the documentation for **{dependency_name}** (version {version}, {language}).
- **Today's date**: {today} (use the current date when running)
- **Latest version**: {latest_version} (from resolve-sources output)
- **Released**: {latest_release_date} (from resolve-sources output)

Extract the highest-value coding rules — the things developers commonly get wrong that aren't caught by standard linters.

### Recency Priority

LLMs are trained on documentation snapshots that are typically 1-2 years old. Whetstone's highest value is catching things the LLM doesn't already know. **Prioritize rules about changes from the last 18 months** — these are the most likely to be missed.

Focus on:
- **API changes since the previous major/minor version** — new recommended patterns, deprecated old ones
- **New defaults** that differ from what older docs/tutorials show
- **Migration paths** that existing LLM training data wouldn't cover
- **Breaking changes** announced for upcoming versions

Deprioritize:
- Patterns that have been stable for 2+ years (LLMs already know these)
- Rules that any developer familiar with the 2024-era version would already follow
- Advice that appears in the majority of tutorials and Stack Overflow answers

### Categories (only these are valid)

| Category | What it catches | Example |
|----------|----------------|---------|
| `migration` | Deprecated APIs that still work but shouldn't be used | Pydantic `.schema()` → `model_json_schema()` |
| `default` | Insecure or slow unless explicitly configured | SQLAlchemy echo=True left in production |
| `convention` | Docs say X, most tutorials/LLMs default to Y | FastAPI sync vs async route handlers |
| `breaking-change` | Will fail in the next major version | Next.js 15 async page params |
| `semantic` | Judgment-based, decomposable into mostly-deterministic signals | Actionable error messages |

### Hard Filters (rules that violate any of these are REJECTED)

1. **Confidence**: If you are not 90%+ confident this rule prevents a real mistake, do not propose it.
2. **Signal requirement**: Every rule MUST have at least one deterministic signal (`ast` or `pattern` strategy). Pure `ai`-only rules are rejected.
3. **Count ceiling**: Maximum 5 rules per dependency. If you cannot rank them, you have not filtered hard enough.
4. **Novelty**: Do NOT propose rules that ruff, biome, or clippy already enforce by default.
5. **Source backing**: Every rule MUST cite a specific URL in the dependency's documentation. No "general best practice" rules.

### What Gets Rejected

- Generic advice ("write clean code", "use meaningful names")
- Things standard linters already catch
- Subjective preferences without documentation backing
- Rules with no testable signal
- Architecture principles that cannot be decomposed into checks
- "Use TypeScript strict mode" (config setting, not a code pattern)

### What Gets Accepted

- Migration footguns (deprecated APIs that still work)
- Non-obvious defaults (insecure/slow unless configured)
- Convention divergence (docs say X, most tutorials/LLMs default to Y)
- Breaking change preparation (will fail in next major version)
- Semantic practices decomposable into mostly-deterministic signals

### Signal Decomposition

Every rule must be decomposed into one or more signals. Each signal has a strategy:

| Strategy | Description | Deterministic? | When to use |
|----------|-------------|----------------|-------------|
| `ast` | Verifiable via syntax tree (AST) analysis | Yes | Function signatures, decorator presence, class inheritance, import patterns |
| `pattern` | Verifiable via text/regex matching | Yes | String literals, config values, naming conventions, comment patterns |
| `lint_proxy` | Maps to an existing linter rule | Yes | When ruff/biome/clippy has a rule but it's not in the default set |
| `ai` | Requires LLM judgment | No | Semantic quality, message clarity — ONLY as supplement to deterministic signals |

Each signal has a weight:
- `required` — rule fails if this signal fires (or doesn't fire, depending on the check)
- `strong` — significant indicator
- `moderate` — supporting evidence

### Output Format

For each proposed rule, output valid YAML following this schema:

```yaml
- id: {dependency}.{rule-name}
  severity: must | should | may
  confidence: high | medium
  category: migration | default | convention | breaking-change | semantic
  description: >
    Concise description using RFC 2119 keywords (MUST, SHOULD, MAY).
  source_url: https://specific-page-in-docs.com/section
  source_quote: >
    Verbatim excerpt from the documentation that supports this rule.
    Keep it short (1-3 sentences) and directly relevant.
  risk: >
    What goes wrong if this rule is ignored (e.g., "Blocks the event loop
    under concurrent load, causing request timeouts").
  linter_gap: >
    Why standard linters (ruff/biome/clippy) don't catch this
    (e.g., "ruff has no rule for async vs sync route handlers").
  status: candidate
  approved: false
  proposed_at: {iso8601_now}
  proposed_by: whetstone-extraction
  signals:
    - id: signal-name
      strategy: ast | pattern | lint_proxy | ai
      description: What this signal checks
      weight: required | strong | moderate
  golden_examples:
    - code: |
        # Correct usage
        ...
      verdict: pass
      reason: Brief explanation of why this passes
    - code: |
        # Incorrect usage
        ...
      verdict: fail
      reason: Brief explanation of why this fails
    - code: |
        # Another correct usage
        ...
      verdict: pass
      reason: Brief explanation of why this passes
```

Provide 3-5 golden examples per rule (mix of pass and fail). These are used for test generation and AI eval calibration.

### Ranking

If you identify more than 5 candidate rules, rank by:
1. **Recency** — does this address a change from the last 18 months that LLMs likely don't know about?
2. **Frequency of mistake** — how often developers get this wrong
3. **Severity of consequence** — what happens when they do
4. **Detectability** — can it be caught with deterministic signals?
5. **Novelty** — is this already caught by standard tooling?

Keep only the top 5. Rules about recent changes (last 18 months) should rank above equally-severe rules about long-standing patterns.

---

## Rule YAML Schema

Rules are stored in `whetstone/rules/{language}/{dependency}.yaml`:

```yaml
source:
  name: dependency-name
  docs_url: https://docs.example.com
  llms_txt: https://docs.example.com/llms.txt    # if available
  version: "1.0.0"
  content_hash: sha256:abc123...
  resolved_at: "2026-03-28T10:00:00Z"
  registry: pypi            # pypi | npm | crates_io | manual

rules:
  - id: dependency.rule-name
    severity: must          # must | should | may
    confidence: high        # high | medium
    category: convention    # migration | default | convention | breaking-change | semantic
    description: >
      Rule description using RFC 2119 keywords.
    source_url: https://docs.example.com/specific-page
    status: approved        # candidate | approved | denied | deprecated
    approved: true
    approved_at: 2026-02-15T12:00:00Z
    proposed_at: 2026-02-15T11:30:00Z
    proposed_by: whetstone-extraction
    signals:
      - id: signal-name
        strategy: ast       # ast | pattern | lint_proxy | ai
        description: What this signal checks
        weight: required    # required | strong | moderate
    golden_examples:
      - code: |
          # pass example
        verdict: pass
        reason: Uses the recommended pattern
      - code: |
          # fail example
        verdict: fail
        reason: Uses the deprecated pattern
```

### Interactive Approval Protocol

Present rules using the **rule card** format. Goal: the user can approve or reject in under 10 seconds per rule.

**Rule Card Format:**

```
[MUST] fastapi.async-routes — high confidence — convention — candidate

  Route handlers MUST use async def.
  Proposed: 2026-03-28T10:00:00Z by whetstone-extraction

  Source: "Use async def for route operations that call async libraries."
          — https://fastapi.tiangolo.com/async/

  Risk:   Blocks the event loop under concurrent load, causing timeouts.
  Gap:    ruff has no rule for async vs sync route handlers.

  Signals: ast (required) — 1/1 deterministic
  Example: async def get_users(): ...  [pass]
           def get_users(): ...        [fail]

  > Approve / Edit / Deny / Skip?
```

Keep each card compact. Only show the most illustrative pass/fail pair inline — full examples live in the YAML.

**Batch Approval:**

Before showing individual cards, offer: **"Approve all N high-confidence rules for {dependency}?"**
- If yes: approve all `confidence: high` rules, then show `confidence: medium` individually
- If no: show all rules individually

**Per-Rule Actions:**

| Action | Effect |
|--------|--------|
| **Approve** | Set `approved: true`, `status: approved`, `approved_at: <now>`, write to YAML |
| **Edit** | User modifies any field, then approve |
| **Deny** | Set `status: denied`, `denied_reason: <reason>`, write to YAML (prevents re-proposal) |
| **Skip** | Keep as `status: candidate` — defer decision to later |

After all rules: "N approved, N denied, N skipped."

### Rule Lifecycle States

Rules persist through these states:

| State | Meaning | Stored in YAML? | Used for generation? |
|-------|---------|------------------|---------------------|
| `candidate` | Proposed, awaiting review | Yes | No |
| `approved` | Reviewed and accepted | Yes | Yes |
| `denied` | Reviewed and rejected | Yes (prevents re-proposal) | No |
| `deprecated` | Previously approved, now invalid | Yes | No |

When saving rules to YAML, always set:
- `status` to the current lifecycle state
- `proposed_at` to the extraction timestamp
- `proposed_by` to `"whetstone-extraction"` (or `"manual"` for hand-written rules)
- `approved_at` only when transitioning to `approved`

Denied rules stay in the YAML with `denied_reason` so they aren't re-proposed on the next extraction run.

---

## Agent Context Format Templates

When `generate-agent-context.py` creates agent context files, it uses this structure:

```markdown
# Project Coding Standards (Auto-generated by Whetstone)
# Last updated: {date}
# Source: whetstone/rules/*.yaml
# Do not edit manually — regenerate with: python3 scripts/generate-agent-context.py

## Patterns to USE

### {Dependency}: {Rule description short}
{Full description}
Source: {source_url}

Do:
\`\`\`{language}
{pass example from golden_examples}
\`\`\`

Don't:
\`\`\`{language}
{fail example from golden_examples}
\`\`\`

## Patterns to AVOID

### {Dependency}: {Deprecated pattern}
{Description of what to avoid and why}
Source: {source_url}

## Conventions

### {Pattern description}
{Description from detected patterns}
```

The file header always includes a generation notice with timestamp and source path so developers know not to edit it manually.

---

## Configuration

Whetstone reads `whetstone/whetstone.yaml` for project settings:

```yaml
languages:
  - python
  - typescript

trigger:
  mode: manual          # manual | session | post-merge | scheduled
  auto_detect_patterns: true

agents:
  - claude.md
  - agents.md
  - cursorrules
  - copilot-instructions.md
  - windsurfrules
  - codex.md

# Source overrides (optional)
sources:
  custom:
    - url: https://team-style-guide.example.com
      name: Team Style Guide
```

---

## File Structure (User's Project)

After running Whetstone, the user's project will contain:

```
whetstone/
  whetstone.yaml              # Config
  rules/
    python/                   # Rule YAML files per language
    typescript/
    rust/
    patterns/                 # Rules from detect-patterns
  evals/
    python/                   # pytest files
    typescript/               # vitest files
    rust/                     # cargo test files
  lint/
    ruff.whetstone.toml       # Ruff overlay
    biome.whetstone.json      # Biome overlay
    clippy.whetstone.toml     # Clippy overlay
  .last-run                   # Timestamp for --since-last-run

# Agent context files at project root
CLAUDE.md
AGENTS.md
.cursorrules
.github/copilot-instructions.md
.windsurfrules
codex.md
```

For detailed reference material, see:
- [Rule YAML schema](references/rule-schema.yaml)
- [Signal strategies guide](references/signal-strategies.md)
- [Extraction prompt details](references/extraction-prompt.md)

---

## Quickstart Recipes

### Local Quickstart

```bash
# 1. Install the skill
npx skills add whetstone

# 2. Ask your agent to bootstrap
"Run whetstone doctor"

# 3. Expected output:
#    - Dependencies detected from manifest files
#    - Documentation sources resolved (with llms.txt where available)
#    - Style patterns mined from history
#    - Rules proposed for approval → you approve/deny each
#    - Agent context files generated (CLAUDE.md, AGENTS.md, etc.)
#    - Test files generated (whetstone/evals/)

# 4. Verify generated tests
pytest whetstone/evals/python/           # Python
npx vitest run whetstone/evals/typescript/  # TypeScript
cargo test --test whetstone              # Rust
```

### CI Quickstart

Add this to `.github/workflows/whetstone.yml`:

```yaml
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
          fail-on: stale
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

Expected: posts a PR comment with health score, drift status, and recommendations.

### Agent Hook Quickstart (Claude Code)

Add to `.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "startup",
      "hooks": [{
        "type": "command",
        "command": "python3 scripts/detect-patterns.py --since-last-run --quiet",
        "async": true,
        "statusMessage": "Whetstone: checking for new style patterns..."
      }]
    }]
  }
}
```

Expected: silently checks for new style patterns on every session start. Only surfaces output if new patterns are found.

### Privacy: Transcript Scanning

`detect-patterns.py` reads agent conversation transcripts from well-known locations under `$HOME`. **By default, scanning is scoped to the current project** — only transcripts whose path contains the project directory name are read.

- **Default:** Project-scoped. Safe to run automatically.
- **`--global-transcripts`:** Scans all projects. Emits a stderr warning. Use only when the user explicitly requests cross-project pattern analysis.
- **No external calls:** All transcript processing is local. No content is sent anywhere.
- **Opt out:** Use `--sources git,pr` to skip transcript scanning entirely.
