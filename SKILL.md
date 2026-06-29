---
name: whetstone
description: >-
  Derives coding rules from trusted sources and dependency documentation,
  submits candidate rules, approves them in bulk, and generates native
  context, tests, and lint/formatter configs.
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

Whetstone turns **trusted sources → approved rules → generated enforcement**.
It derives coding rules from the documentation of your actual dependencies,
your subscribed trusted sources, decomposes them into deterministic checks,
and generates native tests, lint/formatter configs, and agent context files.

## Happy Path

Six commands, in this order:

```bash
wh init             # Bootstrap: detect deps, resolve docs, write extraction handoff
wh extract          # Walk the dependency worklist to find the next candidate
wh extract submit <bundle.yaml>   # Land a candidate bundle as status: candidate
wh rules approve --all --confidence high  # Flip high-confidence candidates to approved
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

# Or bind directly to a linter / formatter / specific test
wh rules add acme.mutable-defaults \
  --description "Mutable default args must be rejected" \
  --lint-tool ruff \
  --lint-code B006 \
  --lang python \
  --dep acme

wh rules add acme.single-quotes \
  --description "Use single quotes in TS" \
  --formatter-tool biome \
  --formatter-option quoteStyle=single \
  --lang typescript \
  --dep acme

wh rules add acme.render-snapshot \
  --description "Renderer output should stay snapshot-covered" \
  --test-runner vitest \
  --test-path tests/render/output.test.ts \
  --test-selector render_output_contract \
  --lang typescript \
  --dep acme

# Bump severity as taste matures
wh rules edit acme.snake-case --severity must

# Bulk: promote every "should" convention rule for a dep to "must"
wh rules edit --all --dep fastapi --category convention --severity must --dry-run
# Remove --dry-run to apply.
```

`--project` routes to the committed layer instead of personal. `wh rules edit` refuses candidate rules — approve first (`wh rules approve <id>`). Use `wh rules remove <id>` to delete one cleanly.

> `--match` mints a raw-regex (`strategy: pattern`) signal, which is **deprecated** and allowlisted only for genuinely text-level checks (string-literal content, naming) where AST adds nothing — like the `snake_case` example above. For anything a linter or formatter already expresses, prefer the `--lint-tool` / `--formatter-tool` / `--test-runner` binding forms.

## Subscribe to custom sources (blogs, wikis, llms.txt, internal docs)

`wh extract` normally walks dependencies detected from manifests. To extract rules from a blog post, a wiki page, an internal style guide, or a custom `llms.txt` endpoint, subscribe it as a **trusted source** — it appears in the extraction worklist alongside detected deps.

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

`--kind` is free-form but conventionally one of `blog`, `official_docs`, `team_guide`, `community`, `custom`. `--lang any` (or omitting `--lang`) scopes the source to all languages. After adding, run `wh init` (or `wh sources verify <name>`) to pull the content, then follow the normal `wh extract` → `wh rules approve` flow. `wh reinit` re-fetches subscribed sources and flags content-hash drift just like it does for detected deps.

## Trusted second-brain / wiki vaults

For richer internal knowledge, Whetstone can also index a local markdown vault as
a **trusted second-brain source graph**. Configure it in
`whetstone/whetstone.yaml`:

```yaml
sources:
  vaults:
    - id: team-brain
      path: docs/brain
      include: ["**/*.md"]
      language: any
      source_kind: second_brain
      authority: reviewed
```

Pages may carry frontmatter like:

```yaml
---
whetstone:
  authority: canonical
  languages: [javascript, html]
  deps: [react]
  upstream:
    - https://react.dev/
  tags: [frontend]
  aliases: [React Patterns]
---
```

Whetstone indexes headings, wikilinks, tags, authority, dependency/language
annotations, and upstream URLs into `whetstone/.state/knowledge-graph.json`.
Those pages then appear in extraction context and worklists as
`source_origin: second_brain_page` entries, with related-page metadata available
to the agent.

## Canonical shareability story

The default shareable entrypoint is a committed `whetstone/whetstone.yaml`.
Teams should be able to copy that file into another repo and keep the same
trusted-source setup. Optional `extends:` pack refs live underneath that file;
they are a composition mechanism, not the primary UX.

Use these commands when inspecting that stack:

```bash
wh config show
wh config validate
```

## Roles

The skill is the front door and owns judgment; it calls the binary for
deterministic work. The user has the final say.

| Task | Handled by | Why |
|------|-----------|-----|
| Dependency detection, source resolution, content fetching | Binary | Deterministic |
| Reading docs + drafting candidate rules | Skill (agent) | Requires judgment |
| Taste / type-aware guidance with no deterministic signal | Skill (agent) | Not gateable; tree-sitter has no type resolution |
| Orchestrating extract → approve → generate | Skill (agent) | Front door |
| Approving candidates | User (via `wh rules approve`) | Policy decision |
| Writing generated tests / lint / context / signal checks | Binary | Deterministic |

## Core Philosophy: High Confidence or Silence

Five rules you trust completely beats fifty you have to review.

- Every **CLI rule** must have a deterministic backing: a `strategy: ast`
  signal (with a real `ast_query`) or a `strategy: lint_proxy` signal, or a
  `formatter` / `tests` / `validators` binding. `strategy: pattern` (raw regex)
  is deprecated; the only allowed form is regex bounded by `ast_scope` inside an
  `ast` signal.
- Guidance that needs **taste or type resolution** (so it can't get a
  deterministic signal) does NOT become a signal-less rule. It lives in this
  skill as agent guidance — see "Taste / type-aware guidance" below. This is how
  "every rule needs a signal" reconciles with the schema treating signals as
  optional: signal-less *judgment* belongs in the skill, not the ruleset.
- Every rule must cite a specific documentation URL.
- If you are not 90%+ confident in a rule, do not submit it.
- Maximum 5 rules per dependency.

### The three-bucket signal audit

Before drafting a rule, place each candidate in exactly one bucket:

1. **Duplicates a linter** (ruff/biome/clippy) → `strategy: lint_proxy` with
   `lint: {tool, code}`; `wh actions lint` emits the native config. For tools
   `lint_proxy` doesn't cover (cargo-audit/RUSTSEC, pip-audit, type-checkers),
   document "use that tool" or bind via `validators: command`. Don't reimplement
   it as regex.
2. **Needs taste or type resolution** → skill guidance, no signal, no CLI rule.
   tree-sitter matches syntax, not types — it can't tell `reqwest::Client` from
   `std::process::Command`, so type-dependent rules belong here.
3. **No linter expresses it AND it's type-independent** → `strategy: ast` with a
   real `ast_query`. The CLI's narrow moat: decorator shape, async-vs-sync form,
   import structure, presence of a node in a clearly-identified construct.

## Taste / type-aware guidance (bucket 2 — lives here, not in the ruleset)

Some of the most valuable dependency advice can't be a CLI rule because enforcing
it would require knowing a value's **type**, and the scanner (tree-sitter) has no
type resolution. These belong in the skill: surface them to the user as guidance
and apply them yourself when reading or writing code, but do **not** submit them
as signal-less rules.

Carry guidance like this in your turn-by-turn judgment:

- **reqwest** — construct clients with an explicit `.timeout(...)`; default is no
  timeout, so a request can hang forever. Also check the response status (e.g.
  `error_for_status()`) before reading the body. (Not a CLI rule: the scanner
  can't confirm the receiver is a `reqwest::Client` vs some other builder, nor
  that a `.timeout()` call is *absent* from the chain.)
- **clap** — prefer the derive API over the builder for typical CLIs. (Not a CLI
  rule: `Command::new(...)` can't be distinguished from `std::process::Command`
  without type resolution.)

When a candidate "rule" turns out to need a type, move it here. Document the
why so the next agent doesn't try to re-add it as a regex.

## Rule lifecycle

```
(agent drafts bundle)
     │
     ▼
wh extract submit  ───▶  status: candidate
                              │
                              ▼
              wh rules approve <id>      status: approved
              wh rules approve --all
```

Only `candidate` and `approved` exist. To retire a rule, prefer
`wh rules remove <id>` so the change goes through the supported CLI surface —
there is no denied/deprecated state to maintain.

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
        strategy: ast
        description: "Route handler declared with sync def instead of async def"
        weight: required
        ast_query: |
          (function_definition name: (identifier)) @fn
      - id: mutable-defaults
        strategy: lint_proxy
        description: "Covered by Ruff"
        weight: required
        lint:
          tool: ruff
          code: B006
    tests:
      - runner: pytest
        path: tests/style/test_fastapi_rules.py
        selector: test_async_routes
    golden_examples: [...]
```

`wh extract submit` refuses to overwrite an existing `whetstone/rules/<lang>/<dep>.yaml`
and fails on any rule-id collision against the current ruleset. Clean up the
colliding file or rename the new candidate, then resubmit.

## Generation

`wh actions` chains three canonical subcommands:

- `wh actions context` — writes `AGENTS.md`, `CLAUDE.md`, `.cursorrules`, etc.
- `wh actions test` — writes pytest / vitest / cargo test scaffolds under `whetstone/evals/`
- `wh actions lint` — writes `ruff.whetstone.toml` / `biome.whetstone.json` /
  `clippy.whetstone.toml` plus formatter-backed overlays like
  `rustfmt.whetstone.toml` when rules opt into safe mechanical formatting
  under `whetstone/lint/`

Compatibility aliases still exist for `wh context`, `wh tests`, and `wh lint`,
but docs and automation should prefer the grouped `wh actions ...` surface.

Run them individually for finer control, or chain them with `wh actions all`.
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

Whetstone is **skill-first with a thin deterministic CLI**. This skill is the
front door — it owns judgment (reading docs, drafting rules, carrying taste/type-
aware guidance) and orchestrates the workflow. The Rust binary (`src/`) is the
deterministic substrate the skill calls: detection, resolution + content-hashing,
schema validation, native lint/test/context generation, the AST/lint scanner, and
the drift gate. "Thin" means role + discipline, not line count — no deterministic
command is removed; only the read-docs → draft-rules loop is skill-driven
(`wh extract` / `wh extract submit` remain as the primitives the skill calls).
Archived Python scripts under `scripts/legacy/` exist only as parity references
for contract tests. The authoritative boundary is
[`planning/skill-cli-boundary.md`](planning/skill-cli-boundary.md).

See [`references/workflow-matrix.md`](references/workflow-matrix.md) for the
command-to-step map, [`references/rule-schema.yaml`](references/rule-schema.yaml)
for the schema, and
[`references/signal-strategies.md`](references/signal-strategies.md) for signal
decomposition guidance.
