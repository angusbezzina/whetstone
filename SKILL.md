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
  version: "0.2.0"
---

# Whetstone

> Sharpen the tools that write your code.

Whetstone derives coding rules from the documentation of your actual dependencies, decomposes them into deterministic checks, and generates native tests, lint configs, and agent context files.

## Activation

Activate when the user says any of: "whetstone", "wh doctor", "extract rules", "update standards", "update rules", "init whetstone", "run whetstone", "check rules", "refresh rules", "generate tests from rules".

If the user says "wh doctor", "doctor", "scan my project", or "bootstrap rules", use the **Doctor** workflow below — it's the fastest path from zero to working rules.

## Happy Path (TL;DR)

Most users need three steps:

1. **Bootstrap**: `wh doctor` — detects deps, resolves docs + changelogs
2. **Extract + Approve**: Read the doctor output's `extraction_context` (includes content and sections per dep), apply the Extraction Prompt for each, present rules for approval
3. **Generate**: `wh context` and `wh tests`

Check health with `wh status` anytime. When deps update, run `wh doctor --changed-only` to re-resolve, then re-extract.

### Repeat Runs

Whetstone caches manifest fingerprints and source resolution results under `whetstone/.state/`. Subsequent runs are faster because unchanged deps are skipped.

| Scenario | Command | What it does |
|----------|---------|-------------|
| Full re-bootstrap | `wh doctor` | Detects all deps, resolves all sources |
| Only changed deps | `wh doctor --changed-only` | Skips cached, resolves stale/missing |
| Resume interrupted run | `wh doctor --resume` | Picks up where last run stopped |
| Force re-resolve | `wh doctor --refresh` | Ignores cache, re-fetches all docs |
| Cap resolution count | `wh doctor --max-deps 5` | Resolves top 5 ranked deps only |
| Retry failed deps | `wh set-sources --retry-failed` | Re-resolves only failed deps |

---

## Architecture

The binary does all deterministic work. The agent does all judgment. The user has final say.

| Task | Handled by | Why |
|------|-----------|-----|
| Dependency detection | **Binary** | Deterministic manifest parsing |
| Source resolution + content fetching | **Binary** | HTTP, caching, parallel fetching |
| Changelog discovery + recency filtering | **Binary** | GitHub API, date parsing |
| Rule extraction | **Agent** | Requires reading and understanding documentation |
| Rule approval | **Agent + User** | Requires judgment and user consent |
| Test + lint config generation | **Binary** | Deterministic codegen from YAML |
| Agent context generation | **Binary** | Deterministic markdown from YAML |
| Health monitoring + CI gating | **Binary** | Deterministic scoring |

## Quick Reference

| Command | Purpose | Output |
|---------|---------|--------|
| `wh doctor` | **One-command bootstrap** | JSON: deps, sources, content, sections, recommendations |
| `wh status` | **Health summary** | JSON: score 0-100, dimensions, recommendations |
| `wh ci` | **CI freshness check** | Exit 0/1, optional PR comment |
| `wh init` | Detect dependencies | JSON: deps list with counts |
| `wh set-sources` | Resolve docs URLs | JSON: source content + cache stats |
| `wh validate` | Check rule YAML schema | Pass/fail per rule |
| `wh context` | Generate agent context files | AGENTS.md, CLAUDE.md, .cursorrules, etc. |
| `wh tests` | Generate test files + lint configs | pytest, vitest, cargo test files |
| `wh patterns` | Mine style patterns | JSON: patterns from transcripts/git/PRs |

All commands support `--json` (auto-enabled when piped) and `--project-dir`.

---

## Content Model

The resolve pipeline fetches documentation through multiple tiers:

| Tier | Source | Confidence | What it provides |
|------|--------|-----------|-----------------|
| 1 | `llms.txt` / `llms-full.txt` | High | Structured, purpose-built for LLMs |
| 2 | Registry README (npm readme, PyPI description, crates.io /readme) | Medium | Package overview, usage patterns |
| 3 | HTML docs → text conversion (scraper extracts main content) | Medium | Full documentation pages |
| 4 | GitHub CHANGELOG.md (recency-filtered to last 18 months) | Medium | Breaking changes, deprecations, new APIs |

Each dependency in the doctor output has:
- `content` — the primary (best-tier) content
- `source_type` — which tier provided it (llms_txt, readme, html_converted, changelog)
- `sections` — array of all available content, labeled by type:

```json
{
  "name": "clap",
  "source_type": "readme",
  "content": "# clap\n\nA full-featured...",
  "sections": [
    {"type": "readme", "content": "...", "url": "https://crates.io/api/v1/crates/clap/4.6.0/readme"},
    {"type": "changelog", "content": "## [4.6.0] - 2026-03-12\n...", "url": "https://raw.githubusercontent.com/...", "versions_covered": "4.5.21–4.6.0"}
  ]
}
```

**When extracting rules, examine each section separately:**
- **Changelog** sections → highest signal for `migration` and `breaking-change` rules
- **README** sections → highest signal for `convention` and `default` rules
- **llms.txt** → comprehensive, use for all categories
- Cross-reference sections for stronger confidence

---

## Workflows

### Doctor (Recommended First Run)

Run when the user says "wh doctor", "doctor", "scan my project", "bootstrap rules". This is the fastest path from zero to working rules.

**Step 1: Run the doctor**

```bash
wh doctor
```

Progress goes to stderr; JSON result to stdout. Review the summary:
- Dependencies found (runtime + dev, per language)
- Sources resolved (how many have content, how many have changelogs)
- Recommendations (what to extract next)

**Step 2: Extract rules from content**

Read `extraction_context.sources` from the doctor output. For each dependency that has content:

1. Read the `sections` array — examine README and changelog separately
2. Apply the **Extraction Prompt** (see below)
3. For changelog content: focus on `migration` and `breaking-change` categories
4. For README content: focus on `convention` and `default` categories
5. Maximum 5 rules per dependency
6. **Prioritize rules about recent changes (last 18 months)**

Every proposed rule MUST include:
- `source_kind` — what kind of source backs it (`official_docs`, `changelog`, `migration_guide`, `blog`, etc.)
- At least one deterministic signal (`ast` or `pattern`)
- A specific `source_url` pointing to the exact documentation

**Step 3: Interactive approval**

Present proposed rules using the **Rule Card** format:

```
[MUST] reqwest.set-timeout — high confidence — default
  Source kind: official_docs
  MUST set an explicit timeout on reqwest clients. Default is no timeout.
  Source: https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout
  Risk:   Hangs indefinitely on unresponsive servers.
  Signals: pattern (required) — 1/1 deterministic
  > Approve / Edit / Deny / Skip?
```

**Batch option**: Offer "Approve all N high-confidence rules for {dep}?" before individual review.

For each rule, the user can:
- **Approve** — write to `whetstone/rules/{language}/{dependency}.yaml` with `approved: true`
- **Edit** — modify severity, signals, or examples, then approve
- **Deny** — skip (optionally note why)
- **Skip** — defer to later

**Step 4: Validate and generate**

```bash
wh validate        # verify rule YAML schema
wh context         # generate agent context files
wh tests           # generate test files + lint configs
```

**Step 5: Confirm health**

```bash
wh status
```

Present the score and next steps. A healthy project scores 80+.

---

### Update (Subsequent Runs)

Run when the user says "update whetstone", "refresh rules", "check for rule updates".

**Step 1: Check for drift**

```bash
wh doctor --changed-only
```

This re-resolves only dependencies whose versions have changed. If nothing changed, inform the user — rules are current.

**Step 2: Extract from changes**

For each dep with new content or changelog entries:
- Focus extraction on **what changed** (new changelog sections, version bumps)
- Propose: new rules, modified rules, rules to deprecate
- Present only the changes, not the full rule set

**Step 3: Approve and regenerate**

Same approval flow, then:

```bash
wh validate
wh context
wh tests
wh status
```

---

### Status

```bash
wh status
```

Five dimensions:
- **Freshness** — days since last extraction
- **Rules count** — total approved rules
- **High confidence ratio** — % backed by high-quality sources
- **Deterministic coverage** — % of signals that don't need AI
- **Pending updates** — deps with version drift

Labels: **Healthy** (80+), **Needs Review** (50-80), **Stale** (<50), **No Rules**.

---

## Extraction Prompt

When extracting rules, follow [references/extraction-prompt.md](references/extraction-prompt.md). Key principles:

- **Recency priority**: Changes from the last 18 months rank highest
- **Hard filters**: 90%+ confidence, ≥1 deterministic signal, max 5 per dep, cite specific doc URL, don't duplicate ruff/biome/clippy
- **Categories**: `migration`, `default`, `convention`, `breaking-change`, `semantic`
- **Signals**: Every rule needs `ast`, `pattern`, or `lint_proxy`. `ai` is supplement only.
- **Match patterns**: Every `pattern` signal SHOULD include a `match` field with a concrete regex. This enables real test generation (without it, tests are TODO stubs).
- **Golden examples**: 3-5 per rule (mix of pass/fail)
- **Source kind**: Every rule MUST include `source_kind` (e.g., `official_docs`, `changelog`, `blog`, `community`)

**Using sections for extraction:**

When a dependency has multiple sections (e.g., README + changelog):
1. Read changelog first — look for deprecations, breaking changes, new patterns
2. Read README — look for conventions, defaults, common misconfigurations
3. Cross-reference: a changelog deprecation confirmed by README guidance is high confidence
4. Set `source_kind` based on which section provided primary evidence

Output valid YAML following the [rule schema](references/rule-schema.yaml).

---

## Rule YAML Format

```yaml
source:
  name: reqwest
  docs_url: "https://docs.rs/reqwest"
  version: "0.12"
  content_hash: "sha256:abc123..."
  resolved_at: "2026-04-05T00:00:00Z"
  registry: crates_io
  content_origin: readme          # How binary fetched it (auto-set)

rules:
  - id: reqwest.set-timeout
    severity: must                 # must | should | may
    confidence: high               # high | medium
    category: default              # migration | default | convention | breaking-change | semantic
    source_kind: official_docs     # What kind of source (agent/user sets this)
    description: >
      MUST set an explicit timeout on reqwest clients.
    source_url: "https://docs.rs/reqwest/latest/..."
    risk: "Hangs indefinitely on unresponsive servers"
    linter_gap: "Clippy doesn't check library defaults"
    approved: true
    status: approved
    proposed_at: "2026-04-05T00:00:00Z"
    proposed_by: whetstone-extraction
    signals:
      - id: client-without-timeout
        strategy: pattern
        description: "Client::new() or ClientBuilder without .timeout()"
        match: 'Client::new\s*\(\)'    # Concrete regex — enables real test generation
        weight: required
    golden_examples:
      - code: |
          let client = Client::builder()
              .timeout(Duration::from_secs(15))
              .build()?;
        verdict: pass
        reason: "Explicit timeout set"
      - code: |
          let client = Client::new();
        verdict: fail
        reason: "No timeout — infinite by default"
```

**`source_kind` values** (open-ended — use any string for custom filtering):

| Value | Use for |
|-------|---------|
| `official_docs` | Vendor documentation, API reference |
| `changelog` | Release notes, CHANGELOG.md entries |
| `migration_guide` | Upgrade/migration documentation |
| `blog` | Blog posts, articles |
| `social` | Twitter/X threads, community posts |
| `community` | Wikis, awesome-lists, StackOverflow |
| `team_guide` | Internal team conventions |
| `manual` | Manually authored by user |

---

## Configuration

`whetstone/whetstone.yaml`:

```yaml
discovery:
  exclude: [node_modules, target, dist]
  include: []

generate:
  formats:
    - agents.md
    - claude.md
    - .cursorrules
```

---

## File Structure

```
whetstone/
  whetstone.yaml              # Config
  rules/
    python/                   # Rule YAML per language
    typescript/
    rust/
  evals/
    python/                   # pytest files
    typescript/               # vitest files
    rust/                     # cargo test files
  lint/
    ruff.whetstone.toml
    biome.whetstone.json
  context/
    AGENTS.md                 # Generated agent context
  .state/                     # Cache (gitignored)
```

For reference material:
- [Rule schema](references/rule-schema.yaml)
- [Signal strategies](references/signal-strategies.md)
- [Extraction prompt](references/extraction-prompt.md)
