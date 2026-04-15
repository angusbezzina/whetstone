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

Activate when the user says any of: "whetstone", "wh doctor", "extract rules", "update standards", "update rules", "init whetstone", "run whetstone", "check rules", "refresh rules", "generate tests from rules", "run evals", "calibrate evals", "promote a rule", "personal rules", "install hooks", "install ci", "scheduled check", "what layer".

If the user says "wh doctor", "doctor", "scan my project", or "bootstrap rules", use the **Doctor** workflow — it's the fastest path from zero to working rules. If the user says "refresh rules" or similar, jump to the **Refresh** workflow. If the user says "personal rules" or asks about rule layers, see **Layers** below.

See [`references/workflow-matrix.md`](references/workflow-matrix.md) for the single source-of-truth table that maps every shipped command to a lifecycle step.

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
| `wh check` | **Deterministic rule scan** | JSON: violations + linter-config gaps |
| `wh eval run` | **Check rules against source** | JSON: violations with file/line/code |
| `wh eval calibrate` | **Validate AI eval prompts** | JSON: agreement rate against golden examples |
| `wh init` | Detect dependencies — or run `--personal` / `--hooks` / `--ci` setup | JSON: deps list with counts, or setup report |
| `wh set-sources` | Resolve docs URLs | JSON: source content + cache stats |
| `wh validate` | Check rule YAML schema | Pass/fail per rule |
| `wh context` | Generate agent context files (`--personal` for personal-only output) | `whetstone/context/*` by default; `whetstone/.personal/context/*` with `--personal` |
| `wh tests` | Generate test files + lint configs (`--personal` for personal-only output) | `whetstone/evals/**` + `whetstone/lint/*` by default; `whetstone/.personal/evals/**` with `--personal` |
| `wh layers` | Show the 4-layer merge summary + per-rule provenance | JSON |
| `wh promote` | Move a rule between layers (`--to personal\|project\|team`) | JSON |
| `wh review` | List rules by status, show one rule, or inspect the review queue / worklist / candidate diff | JSON |
| `wh propose` | `schema`, `diff`, or `import` a structured proposal bundle (replaces hand-authored candidate YAML) | JSON |
| `wh apply` | Apply approve / deny / deprecate / supersede transitions | JSON |
| `wh config` | `show` or `validate` the effective config stack with per-key provenance | JSON |
| `wh bench` | Run the benchmark corpus or snapshot a baseline | JSON |
| `wh eval generate` | Generate AI eval definitions | YAML files for rules with ai signals |
| `wh patterns` | Mine style patterns | JSON: patterns from transcripts/git/PRs |

All commands support `--json` (auto-enabled when piped). Project-scoped commands support `--project-dir`.

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

**Step 2: Walk the dependency worklist**

The doctor writes a per-dependency worklist to `whetstone/.state/extraction-handoff.json`. Inspect it with:

```
wh review worklist                     # every dep
wh review worklist --dep fastapi       # one dep
wh review worklist --lang python       # one language
```

Each entry carries ranked sources, section summaries, the remaining quota (`extraction.max_rules_per_dep`), and an explicit `next_step`. Work top-down: finish `ready_now` deps before touching `resolved_low`, `pending`, or `failed` ones.

**Step 3: Extract rules and emit a proposal bundle**

For each dep you work:

1. Read the `sections` array — examine README and changelog separately
2. Apply the **Extraction Prompt** (see below)
3. For changelog content: focus on `migration` and `breaking-change` categories
4. For README content: focus on `convention` and `default` categories
5. Respect the worklist's `quota.remaining` — never propose more rules than fit
6. **Prioritize rules about recent changes (last 18 months)**

Every proposed rule MUST include:
- `source_kind` — what kind of source backs it (`official_docs`, `changelog`, `migration_guide`, `blog`, etc.)
- At least one deterministic signal (`ast` or `pattern`)
- A specific `source_url` pointing to the exact documentation
- 3 to 5 `golden_examples`, mixing `pass` and `fail` verdicts

Emit a **proposal bundle** matching the schema in `references/proposal-schema.md` (or run `wh propose schema` for the machine-readable version). Save it anywhere — e.g., `whetstone/.state/proposal-{dep}.yaml`.

**Step 4: Diff and import**

```
wh propose diff proposal-fastapi.yaml
```

The diff lists new rule ids, modified candidates, conflicts with approved/denied ids, and advisory deprecations. If it reports `status: conflicts`, **fix the bundle** (rename colliding ids, drop shadowed rules, propose supersession through `wh apply`) before importing.

```
wh propose import proposal-fastapi.yaml
```

The importer writes `whetstone/rules/{language}/{dep}.yaml` with `status: candidate`, `approved: false`, `proposed_at`, and `proposed_by` already populated. **Do not hand-author rule YAML** — the importer is the only supported path.

**Step 5: Interactive approval**

Run `wh review --status=candidate` to list freshly imported rules, then present each using the **Rule Card** format:

```
[MUST] reqwest.set-timeout — high confidence — default
  Source kind: official_docs
  MUST set an explicit timeout on reqwest clients. Default is no timeout.
  Source: https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout
  Risk:   Hangs indefinitely on unresponsive servers.
  Signals: pattern (required) — 1/1 deterministic
  > Approve / Edit / Deny / Skip?
```

**Batch option**: Offer "Approve all N high-confidence rules for {dep}?" before individual review. Use `wh apply --batch <file.json>` with entries `{"rule_id": "...", "action": "approve" | "deny" | "deprecate" | "supersede", ...}`.

For each rule the user can:
- **Approve** — `wh apply <rule-id> --approve`
- **Edit** — fix the bundle YAML for this dep and re-import with `wh propose import <bundle> --overwrite-candidates`
- **Deny** — `wh apply <rule-id> --deny --reason "…"` (reason is mandatory, preserved in the audit log)
- **Skip** — leave as `status: candidate` for later

### Rule lifecycle

Every rule carries a `status` field that records where it is in the review
lifecycle. The transitions:

```
        candidate ──approve──▶ approved ──refresh─▶ deprecated
            │                     │
            └───── deny ─────▶ denied
```

| Status | Meaning | Required extra fields |
|--------|---------|------------------------|
| `candidate` | Proposed by extraction, not yet reviewed | — |
| `approved` | Reviewed and accepted; `approved: true` | `approved_at` |
| `denied` | Reviewed and rejected; kept for audit | `denied_reason` |
| `deprecated` | Previously approved, now superseded or stale | `deprecated_reason` (and `superseded_by` when a replacement exists) |

`wh validate` warns if `approved` and `status` disagree, or if a `denied` /
`deprecated` rule is missing its reason. Keep them consistent.

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

### Refresh (Subsequent Runs)

Run when the user says "refresh rules", "update rules", "check for rule updates", or "update whetstone".

> ⚠️ `wh update` updates the Whetstone **binary** itself (self-update from GitHub Releases). It does NOT touch rules. Use `wh refresh` to re-resolve dependency documentation.

**Step 1: Re-resolve changed sources**

```bash
wh refresh              # re-resolves stale/missing deps, writes refresh-diff.json
wh refresh --check      # same, but exits non-zero when drift exists (for CI)
```

This re-resolves only dependencies whose versions changed. The machine-readable diff lands at `whetstone/.state/refresh-diff.json` — read it to see which deps changed, with before/after source hashes.

If nothing changed, inform the user — rules are current.

**Step 2: Extract from changes**

For each dep with new content or changelog entries:
- Focus extraction on **what changed** (new changelog sections, version bumps)
- Propose: new rules, modified rules, rules to deprecate (set `status: deprecated`)
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

### Layers (Personal, Project, Team, Built-in)

Whetstone resolves rules through four layers. More-specific layers override
broader ones: `personal > project > team > built-in`.

```bash
wh init --personal     # scaffold whetstone/.personal/, auto-gitignore
wh layers              # print JSON summary of each layer + per-rule provenance
wh context --personal  # emit personal-only agent context under .personal/context/
wh tests --personal    # emit personal-only tests under .personal/evals/
wh promote <rule-id> --to project   # promote personal → project
wh promote <rule-id> --to team      # promote project → team (local staging)
```

**Invariants:**

- `whetstone/.personal/` is gitignored and never leaks into committed outputs.
- `wh context` and `wh tests` (no flag) always render from project + team + built-in. The personal layer is stripped before rendering to guarantee personal rules cannot end up in a PR.
- `--personal` renders the personal layer alone to `whetstone/.personal/context|evals/`. Local pytest / vitest / `cargo test` run against both directories; CI only sees committed files.
- `wh promote` is monotonic — you can move a rule "up" (personal → project → team) but not "down". Copying down is a manual override, not a promotion.
- Each layer's `deny:` list removes rules from that layer and every broader layer.

**Team rules via extends:**

```yaml
# whetstone/whetstone.yaml
extends:
  - whetstone:recommended              # embedded built-in — no fetch
  - github.com/acme/whetstone-rules    # git clone into whetstone/.cache/teams/acme/whetstone-rules
```

Git-cloned team rulesets should publish either `whetstone/rules/**` (mirrored
project layout) or `rules/**` (team-only layout). `wh refresh` re-pulls the
cache; otherwise the clone is reused.

**Global personal config:**

`~/.whetstone/config.yaml` (optional) applies defaults to every project:

```yaml
default_formats: [agents.md, .cursorrules]
deny: ["rust.prefer-str-params"]
sources:
  custom:
    - url: https://my-site.example/llms.txt
      name: My reference
```

Project `whetstone.yaml` wins on explicitly-set fields; `deny:` unions.

---

### Triggers (Advisory Automation)

`wh init --hooks` installs advisory hooks that do not block anything:

| Trigger | File | What it does |
|---------|------|--------------|
| Post-merge | `.githooks/post-merge` | After `git pull` / `git merge`, prints a one-line warning if dependency drift is detected. Exit code is always 0. |
| Session start | `.claude/whetstone-session-hook.sh` + merged `.claude/settings.json` | On Claude Code session start, runs `wh status` and surfaces a short label if the project is stale. |
| Cursor | `.cursor/whetstone-session.md` | Documentary advisory — Cursor does not standardise startup hooks, so this is a note for the user. |

`wh init --ci --schedule=<cadence>` writes `.github/workflows/whetstone-check.yml` that runs `wh status` + `wh ci --fail-on=stale` on the chosen cadence (`daily`, `weekly` (default), `biweekly`, `monthly`, or a literal 5-field cron).

---

### Check (Deterministic Rule Scan)

Run when the user says "check rules", "scan for violations", or "run the deterministic checks".

```bash
wh check src --lang rust             # tree-sitter + regex + lint_proxy validation
wh check src --lang python --no-fail # preview results without exiting non-zero
```

`wh check` is the primary enforcement path for deterministic signals:

- `ast_query` signals run through tree-sitter
- `ast_scope` narrows regex checks to specific AST node kinds
- `lint_proxy` signals verify the project's linter configuration and report config gaps

Use `--no-fail` when you want a preview without failing the command.

---

### Eval (AI-assisted / ambiguous checks)

Run when the user says "run evals", "judge ambiguous cases", or "calibrate eval prompts".

```bash
wh eval run --deterministic-only    # Fast: regex checks only, no AI
wh eval run                          # Full: deterministic + AI requests for ambiguous cases
```

The eval runner handles ambiguous or AI-assisted checks. Deterministic enforcement belongs in `wh check`; `wh eval run` layers AI judgment on top when a rule carries `ai_eval` config or `ai` signals.

**For rules with `ai_eval` config:** The runner generates structured eval requests at `whetstone/.state/eval-requests.json`. The agent reads these, judges each code snippet (PASS/FAIL with reason), and writes verdicts to `whetstone/.state/eval-verdicts.json`. Then:

```bash
wh eval run --collect               # Merge agent verdicts into final report
```

**Calibration** validates that AI eval prompts agree with golden examples:

```bash
wh eval calibrate                    # Generate calibration requests
# Agent judges golden examples independently
wh eval calibrate --collect          # Check agreement rate
```

If agreement < 100%, the eval prompt needs revision. This catches model drift and prompt regressions.

**In CI**, use `--deterministic-only` (no agent available):

```bash
wh eval run --deterministic-only     # Exits 0, reports violations in JSON
```

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
    ai/                       # AI eval definitions for rules with ai_eval config
  lint/
    ruff.whetstone.toml
    biome.whetstone.json
  context/
    AGENTS.md                 # Generated agent context
  .state/                     # Cache (gitignored)
    extraction-handoff.json   # written by doctor/refresh; agent reads to extract rules
    refresh-diff.json         # written by refresh; agent reads to focus re-extraction
    eval-requests.json        # written by eval run; agent writes verdicts below
    eval-verdicts.json        # written by agent; read by eval run --collect
    calibration-requests.json # written by eval calibrate
    calibration-verdicts.json # written by agent; read by eval calibrate --collect
```

For reference material:
- [Workflow matrix](references/workflow-matrix.md) — commands, lifecycle stages, artifacts
- [Handoff schema](references/handoff-schema.md) — JSON contracts for every `.state/` file
- [Rule schema](references/rule-schema.yaml)
- [Signal strategies](references/signal-strategies.md)
- [Extraction prompt](references/extraction-prompt.md)
