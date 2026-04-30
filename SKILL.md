---
name: whetstone
description: >-
  Derives coding rules from dependency documentation, submits candidate rules,
  approves them in bulk, and generates native context, tests, and lint configs.
  Use when the user asks to extract rules, update standards, or run whetstone
  commands.
license: MIT
compatibility: Requires the whetstone binary (Rust), git, and internet access for registry lookups.
metadata:
  author: whetstone
  version: "0.3.0"
---

# Whetstone

> Sharpen the tools that write your code.

Whetstone derives coding rules from the documentation of your actual
dependencies, decomposes them into deterministic checks, and generates native
tests, lint configs, and agent context files.

## Happy Path

Six commands, in this order:

```bash
wh init             # Bootstrap: detect deps, resolve docs, write extraction handoff
wh extract          # Walk the dependency worklist to find the next candidate
wh extract submit <bundle.yaml>   # Land a candidate bundle as status: candidate
wh approve --all --confidence high  # Flip high-confidence candidates to approved
wh actions all      # Generate context + tests + lint in one chain
wh scan src/        # Verify source code against approved rules
wh reinit           # Refresh when dependencies change
```

These are the core happy-path commands. Canonical grouped surfaces now live
under `wh rules ...`, `wh sources ...`, and `wh actions ...`; compatibility
aliases still exist for some older grouped names, but docs should prefer the
canonical forms above.

## Mid-turn rule lookup (prefer over reloading AGENTS.md)

When you are about to edit a source file during a turn, **call `wh rules query --file <path>` first** and follow the returned rules. This is cheaper per-turn than scanning the whole committed `AGENTS.md`.

```bash
# Before editing src/services/users.py
wh rules query --file src/services/users.py --json

# Filter to only what must be obeyed
wh rules query --file src/services/users.py --severity must --json

# Everything for one dependency
wh rules query --dep fastapi --json
```

Flags:

- `--file <path>` — infers language from the extension; returns rules for that language.
- `--lang <python|typescript|rust>` — explicit language filter.
- `--dep <name>` — filter to a single dependency.
- `--severity <must|should|may>` — narrow by severity.
- `--full` — include signals and golden examples (useful for debugging a rule; usually unneeded mid-turn).
- `--personal-only` / `--project-only` — layer filter.

The response is a JSON envelope: `{ total, filters, warnings, rules: [...] }`. Each rule carries `id`, `severity`, `description`, `source_url`, `match_patterns`, `layer`, and (with `--full`) signals + examples. Treat `severity: must` rules as non-negotiable; `should` as strong preference; `may` as documented option.

`AGENTS.md` remains the bootstrap context that loads at session start; `wh rules query` is the per-turn lookup that avoids re-scanning it.

### Cheaper bootstrap: `--terse` and per-language sidecars

`wh actions all --terse` (or `wh context --terse`) emits a one-line-per-rule `AGENTS.md` (~50% smaller) that agents can load at session start without consuming much context. Use it when you prefer to rely on `wh rules query` for the details.

When a project has approved rules in more than one language, `wh context` / `wh actions` also emit `AGENTS.<lang>.md` sidecars (one per language) alongside the main `AGENTS.md`. Tools with per-language hooks can point at the narrower file.

## Personal-taste shortcuts

Skip the extract/submit/approve dance for quick personal preferences:

```bash
# Add a personal rule in one command (writes to whetstone/.personal/rules/<lang>/<dep>.yaml as status: approved)
wh rules add acme.snake-case \
  --description "Always use snake_case for Python function names" \
  --match 'def [A-Z]' \
  --lang python \
  --dep acme

# Bump severity as taste matures
wh rules edit acme.snake-case --severity must

# Bulk: promote every "should" convention rule for a dep to "must"
wh rules edit --all --dep fastapi --category convention --severity must --dry-run
# Remove --dry-run to apply.
```

`--project` routes to the committed layer instead of personal. `wh rules edit` refuses candidate rules — approve first (`wh rules approve <id>`). Use `wh rules remove <id>` to delete one cleanly.

## Subscribe to custom sources (blogs, wikis, llms.txt, internal docs)

`wh extract` normally walks dependencies detected from manifests. To extract rules from a blog post, a wiki page, an internal style guide, or a custom `llms.txt` endpoint, subscribe it as a **custom source** — it appears in the extraction worklist alongside detected deps.

```bash
# Personal subscriptions (gitignored — don't leak to teammates)
wh sources add https://blog.example.com/python-tips --name py-tips --lang python --kind blog

# Team subscriptions (committed)
wh sources add https://internal.wiki/style.md --project --name team-style --kind team_guide

# See what's subscribed (both layers)
wh sources list

# Force re-fetch one source (skip a full wh reinit)
wh sources verify py-tips

# Unsubscribe (reports any approved rules that cite the source_url)
wh sources remove py-tips
```

`--kind` is free-form but conventionally one of `blog`, `official_docs`, `team_guide`, `community`, `custom`. `--lang any` (or omitting `--lang`) scopes the source to all languages. After adding, run `wh init` (or `wh sources verify <name>`) to pull the content, then follow the normal `wh extract` → `wh approve` flow. `wh reinit` re-fetches subscribed sources and flags content-hash drift just like it does for detected deps.

## Roles

The binary does deterministic work. The agent does judgment. The user
has the final say.

| Task | Handled by | Why |
|------|-----------|-----|
| Dependency detection, source resolution, content fetching | Binary | Deterministic |
| Reading docs + drafting candidate rules | Agent | Requires judgment |
| Approving candidates | User (via `wh approve`) | Policy decision |
| Writing generated tests / lint / context / signal checks | Binary | Deterministic |

## Core Philosophy: High Confidence or Silence

Five rules you trust completely beats fifty you have to review.

- Every rule **must** have at least one `ast`, `pattern`, or `lint_proxy`
  signal. The `ai` strategy is gone.
- Every rule must cite a specific documentation URL.
- If you are not 90%+ confident in a rule, do not submit it.
- Maximum 5 rules per dependency.

## Rule lifecycle

```
(agent drafts bundle)
     │
     ▼
wh extract submit  ───▶  status: candidate
                              │
                              ▼
              wh approve <id>      status: approved
              wh approve --all
```

Only `candidate` and `approved` exist. To retire a rule, delete the file or
the rule entry directly — there is no denied/deprecated state to maintain.

## Bundles

`wh extract submit` accepts a YAML bundle with this shape:

```yaml
dependency: fastapi
language: python
source:
  name: fastapi
  docs_url: https://fastapi.tiangolo.com
  version: 0.115.0
  registry: pypi
rules:
  - id: fastapi.async-routes
    severity: must
    confidence: high
    category: convention
    description: "..."
    source_url: "..."
    signals:
      - id: sync-def
        strategy: pattern
        description: "pattern"
        weight: required
        match: '\bdef '
    golden_examples: [...]
```

`wh extract submit` refuses to overwrite an existing `whetstone/rules/<lang>/<dep>.yaml`
and fails on any rule-id collision against the current ruleset. Clean up the
colliding file or rename the new candidate, then resubmit.

## Generation

`wh actions` chains three commands:

- `wh context` — writes `AGENTS.md`, `CLAUDE.md`, `.cursorrules`, etc.
- `wh tests` — writes pytest / vitest / cargo test scaffolds under `whetstone/evals/`
- `wh lint` — writes `ruff.whetstone.toml` / `biome.whetstone.json` / `clippy.whetstone.toml`
  under `whetstone/lint/`

Run them individually for finer control, or chain them with `wh actions`.
Every generator accepts `--lang`, `--dry-run`, and `--personal`.

## Personal layer

`wh init --personal` scaffolds `whetstone/.personal/` with its own rules,
context, tests, and lint directories. The directory is auto-added to
`.gitignore`. Personal rules override project rules with the same id; personal
`deny` lists filter merged output only for the local user.

Only the personal + project layers exist. The four-layer merge (plus team
and built-in) is gone.

## Setup extras

- `wh init --hooks` — post-merge + session hooks under `.githooks/`
- `wh init --ci` — schedule `.github/workflows/whetstone-check.yml`
- `wh init --personal` — scaffold `whetstone/.personal/`

## Refresh

`wh reinit` re-resolves only changed deps and writes
`whetstone/.state/refresh-diff.json`. Review the diff, then re-extract any
stale rules with `wh extract submit`.

## Status / health

`wh status` prints a score + dimension breakdown, drift summary, and an
extraction-readiness list. `wh ci` is the lightweight freshness gate for CI.

## Architecture

The Rust binary (`src/`) is the sole runtime. Archived Python scripts under
`scripts/legacy/` exist only as parity references for contract tests.

See [`references/workflow-matrix.md`](references/workflow-matrix.md) for the
command-to-step map, [`references/rule-schema.yaml`](references/rule-schema.yaml)
for the schema, and
[`references/signal-strategies.md`](references/signal-strategies.md) for signal
decomposition guidance.
