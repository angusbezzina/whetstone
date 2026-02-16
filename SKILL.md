---
name: whetstone
description: >-
  Derives coding rules from dependency documentation and developer patterns,
  generates native tests, lint configs, and agent context files. Use when the
  user asks to extract rules, update standards, or run whetstone commands.
license: MIT
compatibility: Requires python3, git, and internet access for registry lookups.
metadata:
  author: whetstone
  version: "0.1.0"
allowed-tools: Bash(python3:*) Bash(git:*) Read Write
---

# Whetstone

> Sharpen the tools that write your code.

Whetstone derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files.

## Activation

Activate when the user says any of: "whetstone", "extract rules", "update standards", "update rules", "init whetstone", "run whetstone", "check rules", "refresh rules", "generate tests from rules".

## Script Paths

All scripts live in this skill's `scripts/` directory. Before running any script, determine the absolute path to this skill's directory (the directory containing this SKILL.md file). Then invoke scripts using that path:

```bash
# Determine SKILL_DIR (the directory containing this SKILL.md)
# Then run scripts as:
python3 "$SKILL_DIR/scripts/detect-deps.py" --project-dir .
```

Throughout this document, `$SKILL_DIR` refers to this skill's installation directory.

## Quick Reference

| Script | Purpose | Input | Output |
|--------|---------|-------|--------|
| `$SKILL_DIR/scripts/detect-deps.py` | Detect dependencies | Manifest files | JSON: deps list |
| `$SKILL_DIR/scripts/resolve-sources.py` | Resolve docs URLs | JSON from detect-deps | JSON: source content |
| `$SKILL_DIR/scripts/detect-patterns.py` | Mine style patterns | Transcripts, git, PRs | JSON: candidate patterns |
| `$SKILL_DIR/scripts/generate-agent-context.py` | Generate agent files | Rule YAML files | CLAUDE.md, AGENTS.md, etc. |
| `$SKILL_DIR/scripts/generate-tests.py` | Generate tests + lint | Rule YAML files | pytest/vitest/cargo tests |

---

## Workflows

### Init (First Run)

Run when the user says "whetstone init", "extract rules", or similar.

**Step 1: Detect dependencies**

```bash
python3 "$SKILL_DIR/scripts/detect-deps.py" --project-dir .
```

Present the findings: "Found N dependencies across [languages]." List the dependencies with name, version, and language. Ask the user which dependencies to extract rules for. Default: all non-dev dependencies.

**Step 2: Resolve documentation sources**

```bash
python3 "$SKILL_DIR/scripts/detect-deps.py" --project-dir . | python3 "$SKILL_DIR/scripts/resolve-sources.py" --deps dep1,dep2,dep3
```

Pass only the user-confirmed dependencies. Present: "Resolved docs for N/M deps, K have llms.txt." For any deps where resolution failed, note why and ask if the user wants to provide a manual docs URL.

**Step 3: Detect style patterns (optional)**

```bash
python3 "$SKILL_DIR/scripts/detect-patterns.py" --project-dir .
```

Present any discovered patterns with evidence (occurrence count, example quotes). Ask the user which patterns to include as rule candidates.

**Step 4: Extract rules**

Read the source content from Step 2 and patterns from Step 3. For each dependency, apply the extraction prompt below. Propose rules following the rule YAML schema. Maximum 5 rules per dependency.

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
python3 "$SKILL_DIR/scripts/generate-agent-context.py" --project-dir .
python3 "$SKILL_DIR/scripts/generate-tests.py" --project-dir .
```

Present summary: "Generated N rules across M deps. Tests: whetstone/evals/. Agent context: CLAUDE.md, AGENTS.md."

**Step 7: Create config (if first run)**

If `whetstone/whetstone.yaml` doesn't exist, create it from `$SKILL_DIR/assets/whetstone.yaml.template` with the detected languages, confirmed agents list, and trigger mode (default: manual).

### Update (Subsequent Runs)

Run when the user says "update whetstone", "refresh rules", "check for rule updates".

**Step 1: Check for drift**

```bash
python3 "$SKILL_DIR/scripts/detect-deps.py" --project-dir . --check-drift
```

Show which dependencies have changed version since last extraction.

**Step 2: Re-resolve changed sources**

```bash
python3 "$SKILL_DIR/scripts/detect-deps.py" --project-dir . | python3 "$SKILL_DIR/scripts/resolve-sources.py" --changed-only --project-dir .
```

Only re-fetch documentation for changed dependencies.

**Step 3: Check for new patterns**

```bash
python3 "$SKILL_DIR/scripts/detect-patterns.py" --project-dir . --since-last-run
```

Show any new patterns discovered since the last run.

**Step 4: Extract and diff**

For changed dependencies, re-run extraction. Compare proposed rules against existing rules. Present only the changes: new rules, modified rules, rules to remove.

**Step 5: Approve changes, regenerate**

Same approval flow as init, but only for changes. After approval, regenerate:

```bash
python3 "$SKILL_DIR/scripts/generate-agent-context.py" --project-dir .
python3 "$SKILL_DIR/scripts/generate-tests.py" --project-dir .
```

### Generate Only

Run when the user says "regenerate tests", "regenerate agent context", or when rules have been manually edited.

```bash
python3 "$SKILL_DIR/scripts/generate-agent-context.py" --project-dir .
python3 "$SKILL_DIR/scripts/generate-tests.py" --project-dir .
```

---

## Extraction Prompt

When extracting rules from dependency documentation, follow these instructions exactly.

### Your Task

You are reading the documentation for **{dependency_name}** (version {version}, {language}). Extract the highest-value coding rules — the things developers commonly get wrong that aren't caught by standard linters.

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
  approved: false
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
    - code: |
        # Incorrect usage
        ...
      verdict: fail
    - code: |
        # Another correct usage
        ...
      verdict: pass
```

Provide 3-5 golden examples per rule (mix of pass and fail). These are used for test generation and AI eval calibration.

### Ranking

If you identify more than 5 candidate rules, rank by:
1. **Frequency of mistake** — how often developers get this wrong
2. **Severity of consequence** — what happens when they do
3. **Detectability** — can it be caught with deterministic signals?
4. **Novelty** — is this already caught by standard tooling?

Keep only the top 5.

---

## Rule YAML Schema

Rules are stored in `whetstone/rules/{language}/{dependency}.yaml`:

```yaml
source:
  name: dependency-name
  docs_url: https://docs.example.com
  llms_txt: https://docs.example.com/llms.txt    # if available
  version: "1.0.0"
  content_hash: abc123...

rules:
  - id: dependency.rule-name
    severity: must          # must | should | may
    confidence: high        # high | medium
    category: convention    # migration | default | convention | breaking-change | semantic
    description: >
      Rule description using RFC 2119 keywords.
    source_url: https://docs.example.com/specific-page
    approved: true
    approved_at: 2026-02-15T12:00:00Z
    signals:
      - id: signal-name
        strategy: ast       # ast | pattern | lint_proxy | ai
        description: What this signal checks
        weight: required    # required | strong | moderate
    golden_examples:
      - code: |
          # pass example
        verdict: pass
      - code: |
          # fail example
        verdict: fail
```

### Interactive Approval Protocol

When presenting rules for approval:

1. Show one rule at a time (or group by dependency if the user prefers)
2. For each rule, display:
   - **ID**: `dependency.rule-name`
   - **Severity**: MUST / SHOULD / MAY
   - **Category**: migration / default / convention / breaking-change / semantic
   - **Confidence**: high / medium
   - **Description**: The rule text
   - **Source**: Link to documentation
   - **Signals**: How it will be checked (strategy + description)
   - **Examples**: Pass and fail code blocks
3. Ask: "Approve, edit, deny, or skip?"
4. On **approve**: Set `approved: true`, `approved_at: <now>`, write to YAML file
5. On **edit**: Let user modify any field, then save
6. On **deny**: Do not save; optionally record denial reason
7. On **skip**: Leave for later; do not save

After all rules are reviewed, show a summary: N approved, N denied, N skipped.

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
