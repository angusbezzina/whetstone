---
name: whetstone
description: >-
  Derives coding rules from dependency documentation and developer patterns,
  generates native tests, lint configs, and agent context files. Use when the
  user asks to extract rules, update standards, or run whetstone commands.
license: MIT
compatibility: Requires the whetstone binary (Rust), git, and internet access for registry lookups.
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

1. **Bootstrap**: `whetstone doctor` — detects deps, resolves docs
2. **Extract + Approve**: Read the doctor's `extraction_context`, apply the Extraction Prompt below for each source, present rules for approval using the Rule Card format
3. **Generate**: `whetstone generate-context` and `whetstone generate-tests`

After that, check health with `whetstone status` anytime.
When deps update, run `whetstone doctor --changed-only` to resolve only changed deps, then re-extract.

### Repeat Runs

Whetstone caches manifest fingerprints and source resolution results under `whetstone/.state/`. Subsequent runs are faster because unchanged deps are skipped.

| Scenario | Command | What it does |
|----------|---------|-------------|
| Full re-bootstrap | `whetstone doctor` | Detects all deps, resolves all sources |
| Only changed deps | `whetstone doctor --changed-only` | Skips cached, resolves stale/missing |
| Resume interrupted run | `whetstone doctor --resume` | Picks up where last run stopped |
| Force re-resolve | `whetstone doctor --refresh` | Ignores cache, re-fetches all docs |
| Cap resolution count | `whetstone doctor --max-deps 5` | Resolves top 5 ranked deps only |
| Extract ready subset | `whetstone doctor --ready-only` | Hands off only extraction-ready deps |
| Retry failed deps | `whetstone resolve-sources --retry-failed` | Re-resolves only failed deps |

See the **Doctor** workflow below for the detailed version.

---

## Binary Usage

The `whetstone` binary is the primary interface. When using Whetstone as an agent skill, the agent invokes the binary directly. All commands accept `--project-dir` (default: `.`).

## Quick Reference

| Command | Purpose | Input | Output |
|---------|---------|-------|--------|
| `whetstone doctor` | **One-command bootstrap** | Project dir | JSON: extraction context |
| `whetstone status` | **Project health summary** | Rule YAML files | JSON: health dimensions |
| `whetstone ci-check` | **CI freshness check** | Project dir | JSON: CI outputs |
| `whetstone detect-deps` | Detect dependencies | Manifest files | JSON: deps list |
| `whetstone resolve-sources` | Resolve docs URLs | JSON from detect-deps | JSON: source content |
| `whetstone generate-context` | Generate agent files | Rule YAML files | AGENTS.md, CLAUDE.md, etc. |
| `whetstone generate-tests` | Generate tests + lint | Rule YAML files | pytest/vitest/cargo tests |

### Common Flags

All scripts accept `--project-dir` (default: `.`). User-facing scripts support these output modes:

| Flag | Behavior | Available in |
|------|----------|-------------|
| `--json` | JSON only to stdout (suppress human output) | doctor, status, ci-check |
| `--score` | Just the numeric score + label | status |
| `--pr-comment` | GitHub PR comment markdown | ci-check |
| `--changed-only` | Only process deps with drift | detect-deps, doctor, ci-check |
| `--dry-run` | Preview without writing files | generate-context, generate-tests |
| `--check-drift` | Include drift info in output | detect-deps |
| `--incremental` | Fingerprint manifests, persist inventory | detect-deps |
| `--resume` | Skip already-resolved deps | resolve-sources, doctor |
| `--retry-failed` | Re-resolve only failed deps | resolve-sources |
| `--force-refresh` | Ignore cache, re-fetch all | resolve-sources |
| `--refresh` | Force re-resolve even cached deps | doctor |
| `--ttl N` | Cache TTL in seconds (default: 7 days) | resolve-sources |
| `--workers N` | Parallel resolution workers (default: auto, capped) | resolve-sources |
| `--max-deps N` | Cap how many deps to resolve | doctor |
| `--ready-only` | Only hand off extraction-ready deps | doctor |
| `--extraction-ready` | List deps in extraction_ready state | status |

All commands output JSON to stdout. Pattern detection (`detect-patterns`) is planned for a future release.

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
| Dependency detection | **Binary** | Deterministic manifest parsing |
| Source resolution | **Binary** | Deterministic registry API calls |
| Rule extraction | **Agent** | Requires reading and understanding documentation |
| Rule approval | **Agent + User** | Requires judgment and user consent |
| Test generation | **Binary** | Deterministic code generation from approved YAML |
| Agent context generation | **Binary** | Deterministic markdown generation from approved YAML |
| Health monitoring | **Binary** | Deterministic metric computation |
| CI gating | **Binary** | Deterministic pass/fail decision |

The binary handles all deterministic work. The agent brings judgment to rule extraction and approval.

---

## Workflows

### Doctor (Recommended First Run)

Run when the user says "whetstone doctor", "doctor", "scan my project", "bootstrap rules", or when they want the fastest path from zero to working rules. This is the **recommended** entry point — it chains detect → resolve → patterns → extract → generate in one flow.

**Step 1: Run the doctor orchestrator**

```bash
whetstone doctor
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
whetstone generate-context
whetstone generate-tests
```

**Step 5: Create config (if first run)**

If `whetstone/whetstone.yaml` doesn't exist, create it from `assets/whetstone.yaml.template` with the detected languages, confirmed agents list, and trigger mode (default: manual).

**Step 6: Summary**

Present a final summary:
- Rules: N approved across M dependencies + K style patterns
- Tests: paths to generated test files
- Agent context: which files were generated

**Next:** "Run your tests to verify: `pytest whetstone/evals/python/`" (or `npx vitest` / `cargo test` for the relevant language). Then run `whetstone status` to confirm health.

---

### Init (First Run — Step-by-Step)

Run when the user says "whetstone init", "extract rules", or wants more control than the Doctor workflow provides. The Doctor workflow above is recommended for most users — use Init when you need to customize each step.

**Step 1: Detect dependencies**

```bash
whetstone detect-deps
```

Present the findings: "Found N dependencies across [languages]." List the dependencies with name, version, and language. Ask the user which dependencies to extract rules for. Default: all non-dev dependencies.

**Step 2: Resolve documentation sources**

```bash
whetstone detect-deps | whetstone resolve-sources --deps dep1,dep2,dep3
```

Pass only the user-confirmed dependencies. Present: "Resolved docs for N/M deps, K have llms.txt." For any deps where resolution failed, note why and ask if the user wants to provide a manual docs URL.

**Step 3: Detect style patterns (optional)**

Pattern detection is planned for a future release. Skip this step.

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
whetstone generate-context
whetstone generate-tests
```

Present summary: "Generated N rules across M deps. Tests: whetstone/evals/. Agent context: CLAUDE.md, AGENTS.md."

**Step 7: Create config (if first run)**

If `whetstone/whetstone.yaml` doesn't exist, create it from `assets/whetstone.yaml.template` with the detected languages, confirmed agents list, and trigger mode (default: manual).

**Next:** "Run your tests to verify: `pytest whetstone/evals/python/`" (or the equivalent for your language). Then run `whetstone status` to confirm health.

### Update (Subsequent Runs)

Run when the user says "update whetstone", "refresh rules", "check for rule updates".

By default, update only processes dependencies that have changed (diff-only mode). Use this unless the user explicitly requests a full re-extraction.

**Step 1: Check for drift (changed deps only)**

```bash
whetstone detect-deps --changed-only
```

This outputs only dependencies whose versions have drifted since last extraction. If no drift is found, inform the user and suggest running `whetstone status` instead. For a full check, use `--check-drift` (shows drift info but still outputs all deps).

**Step 2: Re-resolve changed sources only**

```bash
whetstone detect-deps --changed-only | whetstone resolve-sources --changed-only
```

Only re-fetches documentation for dependencies with version drift AND content changes. This is fast and avoids unnecessary network calls.

**Step 3: Check for new patterns**

Pattern detection is planned for a future release. Skip this step.

**Step 4: Extract and diff**

For changed dependencies, re-run extraction. Compare proposed rules against existing rules. Present only the changes: new rules, modified rules, rules to remove.

**Step 5: Approve changes, regenerate**

Same approval flow as init, but only for changes. After approval, regenerate:

```bash
whetstone generate-context
whetstone generate-tests
```

**Next:** "Run updated tests to verify: `pytest whetstone/evals/python/`". Then run `whetstone status` to confirm the drift is resolved.

### Status

Run when the user says "whetstone status", "check health", "how are my rules", or similar.

```bash
whetstone status
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
whetstone generate-context
whetstone generate-tests
```

**Next:** "Run tests to verify the regenerated outputs." Then run `whetstone status` to check overall health.

---

## Extraction Prompt

When extracting rules from dependency documentation, follow the full extraction prompt at [references/extraction-prompt.md](references/extraction-prompt.md). Key principles:

- **Recency priority**: Focus on changes from the last 18 months that LLMs were likely not trained on
- **Hard filters**: 90%+ confidence, at least one deterministic signal, max 5 per dep, must cite specific doc URL, must not duplicate ruff/biome/clippy
- **Categories**: `migration`, `default`, `convention`, `breaking-change`, `semantic`
- **Signals**: Each rule needs `ast`, `pattern`, or `lint_proxy` signals. `ai` is supplement only.
- **Golden examples**: 3-5 per rule (mix of pass/fail), used for test generation

Output valid YAML following the [rule schema](references/rule-schema.yaml). See [signal strategies](references/signal-strategies.md) for decomposition guidance.

---

## Interactive Approval

Present rules using a compact **rule card** format. Goal: approve/reject in under 10 seconds per rule.

```
[MUST] fastapi.async-routes — high confidence — convention — candidate
  Route handlers MUST use async def.
  Source: https://fastapi.tiangolo.com/async/
  Risk:   Blocks the event loop under concurrent load.
  Signals: ast (required) — 1/1 deterministic
  > Approve / Edit / Deny / Skip?
```

**Batch option**: Offer "Approve all N high-confidence rules for {dep}?" before individual review.

**Actions**: Approve (write to YAML), Edit (modify then approve), Deny (prevents re-proposal), Skip (defer).

**Lifecycle**: `candidate` → `approved` | `denied` | `deprecated`. Denied rules stay in YAML to prevent re-proposal.

Save approved rules to `whetstone/rules/{language}/{dependency}.yaml`.

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
  .state/                     # Pipeline cache (gitignored)
    manifests.json            # Manifest fingerprints
    inventory.json            # Dependency lifecycle state
    source-cache.json         # Source resolution cache
    refresh-log.json          # Cache invalidation log
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

## Quickstart

### Local

```bash
# Install (pick one)
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | bash
# OR: cargo install --git https://github.com/angusbezzina/whetstone.git

# Bootstrap rules
whetstone doctor          # detect deps, resolve docs, prepare extraction context
# Agent extracts rules, user approves/denies each
whetstone generate-context  # generate CLAUDE.md, AGENTS.md, etc.
whetstone generate-tests    # generate test files

# Verify
pytest whetstone/evals/python/           # Python
npx vitest run whetstone/evals/typescript/  # TypeScript
cargo test --test whetstone              # Rust
```

### CI

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
      - uses: angusbezzina/whetstone@main
        with:
          fail-on: stale
          github-token: ${{ secrets.GITHUB_TOKEN }}
```
