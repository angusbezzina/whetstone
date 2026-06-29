# Whetstone Overview

> **Last updated:** 2026-05-06
> **Version:** `0.3.0` tagged · `[Unreleased]` on `main` (queued for `0.4.0`)
> **Related reading:** [`SKILL.md`](../SKILL.md) · [`references/workflow-matrix.md`](../references/workflow-matrix.md) · [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd)

---

## What Whetstone is

Whetstone is the **source-to-rule layer** for your codebase. In practice, it
lets a project encode **taste** as:

> trusted sources → approved rules → generated agent context + enforcement + health reporting

It reads your dependency manifests (plus any extra trusted sources you
subscribe to), fetches the real docs and changelogs, lets an agent draft
high-confidence coding rules from those sources, and lets users hand-author
strict rules when they already know the exact behavior they want.

Once approved, those rules drive the outputs that actually matter:

- **Agent context** (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`, …)
- **Lint / formatter overlays** (`ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`, `rustfmt.whetstone.toml`, …)
- **Runnable tests** (pytest, vitest, cargo test)
- **Repo-health signals** (`wh status`, `wh report`, `wh debt`)

It does **not** replace ruff / biome / clippy or your formatter. It decides
which source-backed rules are worth encoding into those tools, fills the gap
between what they catch and what the docs say, and only uses
formatter-backed enforcement where the rule maps cleanly to a safe mechanical
rewrite.

The **agent is the LLM** — no API key, no LLM client in the binary. Whetstone
gives your existing agent deterministic JSON oracles (`wh extract`,
`wh rules query`, `wh scan`, `wh status`, `wh debt`, `wh config show`) to
reason against.

---

## Who does what

Whetstone is **skill-first with a thin deterministic CLI**: the skill (agent) is
the front door and owns judgment; it *calls* the binary for deterministic work.
"Thin" means role + discipline, not line count — no deterministic command is
removed; only the read-docs → draft-rules loop is skill-driven. The authoritative
split is [`planning/skill-cli-boundary.md`](./skill-cli-boundary.md).

| Actor | Role |
|-------|------|
| **Skill / Agent** (Claude, Cursor, Codex, …) | Front door. Reads fetched source material, drafts candidate rules, carries taste / type-aware guidance that can't be deterministically enforced, orchestrates the workflow, and calls binary oracles mid-turn. |
| **Binary** (Rust, this repo) | Deterministic substrate the skill calls: detects manifests, fetches docs + content-hashes, validates YAML, scans source for violations, writes every artifact, and computes repo-health/drift views. |
| **User** | Chooses trusted sources, approves rules, hand-authors strict rules, and decides what counts as project taste. |

---

## Taste model

Whetstone treats taste as **evidence-backed policy**, not free-floating style
preference.

1. **Choose trusted sources** — official docs, changelogs, internal guides, blogs, `llms.txt`, and other sources you actually trust.
2. **Derive or author rules** — let the agent extract candidate rules from those sources, or hand-author rules directly when the policy is already obvious.
3. **Approve the ruleset** — the repo's taste becomes the approved ruleset, not the raw source list.
4. **Generate guidance + enforcement** — context files, lint surfaces, tests, and formatter-backed enforcement where the mapping is safe and explicit.
5. **Measure project health** — adherence, drift, and deterministic debt all report how well the repo is living up to that taste.

This is why **sources** matter so much: taste is upstream of rules. The default
sharing primitive should therefore be a canonical **`whetstone/whetstone.yaml`**
that teams can copy between projects, with optional **source/config packs**
layered underneath it for reuse.

---

## The loop

```
┌──────────────────────────────────────────────────────────────────────┐
│  1.  BOOTSTRAP                                     [Binary]           │
│      wh init                                                          │
│      → detect manifests → resolve dep docs + changelogs               │
│      → fetch any subscribed trusted sources                           │
│      → write .state/extraction-handoff.json                           │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  2.  EXTRACT                                       [Agent]            │
│      wh extract                     (top dep/source + ranked content) │
│      ... agent reads the source material, drafts candidate rules ...  │
│      wh extract submit <bundle.yaml>                                  │
│      → whetstone/rules/<lang>/<dep>.yaml   (status: candidate)        │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  3.  APPROVE                                       [User + Agent]     │
│      wh rules approve <rule-id>                                       │
│      wh rules approve --all [--dep X] [--confidence high]             │
│      → status: candidate → approved                                   │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  4.  GENERATE                                      [Binary]           │
│      wh actions all [--terse]                                         │
│      → whetstone/context/   AGENTS.md + per-language AGENTS.<lang>.md │
│      → whetstone/evals/     pytest / vitest / cargo test scaffolds    │
│      → whetstone/lint/      lint + formatter overlays                 │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  5.  VERIFY                                        [Binary]           │
│      wh scan src/                                                     │
│      → tree-sitter AST + AST-scoped regex + lint-proxy verification   │
│      → exit 0 (clean) or exit 1 (violations)                          │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  6.  MONITOR                                       [Binary]           │
│      wh status   rule_system_score + adherence_score + trend          │
│      wh report   one-page markdown narrative (PR-friendly)            │
│      wh debt     deterministic debt triage for key hotspots           │
│      wh ci       freshness gate for CI pipelines                      │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  7.  MAINTAIN                                      [Binary]           │
│      wh reinit   re-resolve deps + sources                            │
│      → .state/refresh-diff.json                                       │
│      Drift detected? Loop back to step 2.                             │
└──────────────────────────────────────────────────────────────────────┘
```

### Auxiliary flows

**Subscribe to trusted sources**

```bash
wh sources add https://blog.example.com/py --name py-tips --lang python --kind blog
wh sources add https://team.internal/style --project --kind team_guide
wh sources list
wh sources verify py-tips
wh sources remove py-tips
```

**Hand-author strict rules**

```bash
wh rules add acme.no-print \
  --description "Never call print() in production code" \
  --match 'print\s*\(' --lang python

wh rules edit acme.no-print --severity must
wh rules edit --all --dep fastapi --category convention --severity must --dry-run
```

**Inspect config / packs**

```bash
wh config show
wh config validate
```

**Mid-turn JIT rule lookup**

```bash
wh rules query --file src/services/users.py --severity must --json
wh rules query --dep fastapi --full
```

---

## Commands

The complete canonical surface, grouped by concern. All commands accept
`--json` and `--project-dir`. Full artifact I/O lives in
[`references/workflow-matrix.md`](../references/workflow-matrix.md).

### Bootstrap & maintenance

| Command | Purpose |
|---------|---------|
| `wh init` | Detect deps, resolve docs, fetch subscribed sources, write extraction handoff. |
| `wh reinit` | Re-resolve deps and sources; flag version/content-hash drift. |
| `wh set-sources` | Lower-level resolution-only slice of init. Usually implicit. |

### Authoring

| Command | Purpose |
|---------|---------|
| `wh extract` | Print the next worklist dependency or subscribed source. |
| `wh extract submit <bundle.yaml>` | Write source-derived rules as `status: candidate`. |
| `wh rules approve <rule-id>` / `--all` | Flip candidates to approved. |
| `wh rules add <id>` | Hand-author a strict rule directly. |
| `wh rules edit <id>` / `--all` | Bump `severity` / `confidence` on approved rules. |

### Subscribing to sources

| Command | Purpose |
|---------|---------|
| `wh sources add <url>` | Subscribe a trusted source. |
| `wh sources list` | Cross-layer inventory of subscribed sources. |
| `wh sources remove <target>` | Unsubscribe by URL or name. |
| `wh sources verify <target>` | Force re-fetch one source without full `wh reinit`. |

### Generation

| Command | Purpose |
|---------|---------|
| `wh actions context` | Agent context files under `whetstone/context/`. |
| `wh actions test` | Test scaffolds under `whetstone/evals/`. |
| `wh actions lint` | Lint + formatter overlays under `whetstone/lint/`. |
| `wh actions all` | Chains context + tests + lint. |

### Enforcement & monitoring

| Command | Purpose |
|---------|---------|
| `wh scan <path>` | Deterministic rule scan (tree-sitter + regex + lint_proxy). |
| `wh validate` | Schema + fixture validation. |
| `wh status` | Rule-system health + adherence score. |
| `wh report` | One-page markdown summary. |
| `wh debt` | Deterministic technical-debt triage. |
| `wh ci` | Freshness gate with optional PR comment. |

### Inspection & self-update

| Command | Purpose |
|---------|---------|
| `wh rules query` | JIT rule lookup for agents. |
| `wh config show / validate` | Inspect canonical `whetstone/whetstone.yaml`, imported packs, and effective per-key provenance. |
| `wh review` | Read-only rule inspection / worklist view. |
| `wh update` | Self-update the binary from GitHub Releases. |

---

## Ship status

### 0.3.0 — tagged, shipped

The lean refactor. Surface collapsed from ~20 commands to a seven-command happy
path. Deterministic `wh scan` (née `wh check`) with tree-sitter + AST-scoped
regex + lint-proxy. Two-layer merge (personal + project). Pre-push hook
enforcing CI-parity gates locally.

### [Unreleased] — on `main`, queued for `0.4.0`

- `wh rules query`
- `wh context --terse` + per-language sidecars
- `wh rules add` / `wh rules edit`
- `wh sources add / list / remove / verify`
- `wh config show / validate`
- imported config packs under canonical `whetstone/whetstone.yaml`
- formatter-backed enforcement overlays
- `adherence_score` in `wh status`
- `wh report`
- smarter `wh reinit`

### Near-term

| Item | Tracking |
|------|----------|
| Cut 0.4.0 | TBD |
| Skill ↔ CLI boundary alignment (`planning/skill-cli-boundary.md`) | `whetstone-4xw` |
| Archived-planning cleanup | TBD |

### Future concerns

- tech-debt quantification
- local MCP wrapper around `wh rules query` / `wh scan`
- shared registries / publishing ecosystem

See [`planning/platform-registry.md`](./platform-registry.md) and
[`references/platform-registry-api.md`](../references/platform-registry-api.md)
for the current platform/registry design.

---

## Design principles

1. **High confidence or silence.**
2. **Taste starts with sources.**
3. **CLI as structured oracle.**
4. **The agent IS the LLM.**
5. **Complement, don't compete.**
6. **Generated outputs are the product.**
7. **Incremental by default.**
8. **Lean over comprehensive.**

---

## Supported languages

| Language | Manifest | Registry | Test output | Lint output |
|----------|---------|----------|-------------|-------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff |
| TypeScript | `package.json` | npm | vitest | biome |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy |

---

## Key files

| File | Purpose |
|------|---------|
| `SKILL.md` | Agent skill workflow |
| `references/rule-schema.yaml` | Rule YAML format |
| `references/workflow-matrix.md` | Shipped command matrix with artifact I/O |
| `references/handoff-schema.md` | `.state/*.json` contracts |
| `planning/whetstone-logic-flow.mmd` | Visual flow chart |
| `planning/shared-config-packs.md` | Canonical shareability + pack design |
| `planning/formatter-enforcement.md` | Formatter-backed enforcement contract |
| `planning/platform-registry.md` | Epic 4 platform + registry design |
| `references/platform-registry-api.md` | Future registry API / publish contract sketch |

---

*Whetstone sharpens the tools that write your code.*
