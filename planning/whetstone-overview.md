# Whetstone Overview

> **Last updated:** 2026-04-21
> **Version:** `0.3.0` tagged · `[Unreleased]` on `main` (queued for `0.4.0`)
> **Related reading:** [`SKILL.md`](../SKILL.md) · [`references/workflow-matrix.md`](../references/workflow-matrix.md) · [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd)

---

## What Whetstone is

Whetstone is the **rule-intelligence layer** for your codebase. It reads your dependency manifests (plus any extra sources you subscribe to), fetches the real docs, lets an agent draft high-confidence coding rules, and — once you approve them — generates the three things that actually enforce them:

- **Agent context** (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`, …) so Claude / Cursor / Codex write code the way your docs say they should.
- **Linter overlays** (`ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`) so ruff / biome / clippy catch the rules they can.
- **Runnable tests** (pytest, vitest, cargo test) for the rules linters can't.

It does **not** replace ruff / biome / clippy; it fills the gap between what they catch and what the docs say. And the **agent is the LLM** — no API key, no LLM client in the binary. Whetstone gives your existing agent deterministic JSON oracles (`wh extract`, `wh rules query`, `wh check`, `wh status`) to reason against, adding zero incremental inference cost.

---

## Who does what

| Actor | Role |
|-------|------|
| **Binary** (Rust, this repo) | Detects manifests, fetches docs, validates YAML, scans source for violations, writes every artifact. Always deterministic. |
| **Agent** (Claude, Cursor, Codex, …) | Reads fetched docs, drafts candidate rules, calls binary oracles mid-turn. The LLM you're already paying for. |
| **User** | Approves rules, bumps severity as taste matures, subscribes to extra sources. Final say. |

---

## The loop

```
┌──────────────────────────────────────────────────────────────────────┐
│  1.  BOOTSTRAP                                     [Binary]           │
│      wh init                                                          │
│      → detect manifests → resolve dep docs + changelogs               │
│      → fetch any subscribed custom sources                            │
│      → write .state/extraction-handoff.json                           │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  2.  EXTRACT                                       [Agent]            │
│      wh extract                     (top dep/source + ranked content) │
│      ... agent reads the docs, drafts a candidate bundle YAML ...     │
│      wh extract submit <bundle.yaml>                                  │
│      → whetstone/rules/<lang>/<dep>.yaml   (status: candidate)        │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  3.  APPROVE                                       [User + Agent]     │
│      wh approve <rule-id>                                             │
│      wh approve --all [--dep X] [--confidence high]                   │
│      → status: candidate → approved                                   │
│      (Denial = delete the rule file. No separate deny command.)       │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  4.  GENERATE                                      [Binary]           │
│      wh actions [--terse]   (chains wh context + wh tests + wh lint)  │
│      → whetstone/context/   AGENTS.md + per-language AGENTS.<lang>.md │
│      → whetstone/evals/     pytest / vitest / cargo test scaffolds    │
│      → whetstone/lint/      ruff / biome / clippy overlays            │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  5.  VERIFY                                        [Binary]           │
│      wh check src/                                                    │
│      → tree-sitter AST + AST-scoped regex + lint-proxy verification   │
│      → exit 0 (clean) or exit 1 (violations)                          │
│      The agent's self-check before declaring a task done.             │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  6.  MONITOR                                       [Binary]           │
│      wh status   rule_system_score + adherence_score + trend          │
│      wh report   one-page markdown narrative (PR-friendly)            │
│      wh ci       freshness gate for CI pipelines                      │
└──────────────────────────┬───────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  7.  MAINTAIN                                      [Binary]           │
│      wh reinit   re-resolve deps (version + content-hash drift)       │
│      → .state/refresh-diff.json                                       │
│         carries re_extraction_candidates + canned extraction_prompt   │
│      Drift detected? Loop back to step 2.                             │
└──────────────────────────────────────────────────────────────────────┘
```

### Auxiliary flows

Three first-class side-paths attach to the loop rather than running through it end-to-end.

**Subscribe to custom rule sources** — blogs, wikis, internal style guides, arbitrary `llms.txt` endpoints. Subscribed sources appear in the `wh extract` worklist alongside detected deps; `wh reinit` flags content-hash drift on them too.

```
wh source add https://blog.example.com/py --name py-tips --lang python --kind blog
  → whetstone/.personal/config.yaml  (gitignored — personal, the default)
wh source add https://team.internal/style --project --kind team_guide
  → whetstone/whetstone.yaml         (committed — shared)
wh source list            cross-layer inventory
wh source fetch py-tips   force re-fetch one source (skip full wh reinit)
wh source remove py-tips  unsubscribe; reports any approved rules citing it
```

**Personal-taste shortcuts** — skip extract/submit/approve when you already know the rule you want.

```
wh rule add acme.no-print \
  --description "Never call print() in production code" \
  --match 'print\s*\(' --lang python
  → whetstone/.personal/rules/python/acme.yaml   (status: approved)

wh rule edit acme.no-print --severity must
wh rule edit --all --dep fastapi --category convention --severity must --dry-run
  → in-place severity / confidence mutation on approved rules
```

**Mid-turn JIT rule lookup** — the agent calls this instead of re-scanning `AGENTS.md` on every edit.

```
wh rules query --file src/services/users.py --severity must --json
wh rules query --dep fastapi --full
  → JSON: { total, filters, rules: [ { id, severity, match_patterns, … } ] }
```

The mermaid at [`planning/whetstone-logic-flow.mmd`](./whetstone-logic-flow.mmd) shows the graph view with the CI path and all side-channels.

---

## Commands

The complete canonical surface, grouped by concern. No aliases. All commands accept `--json` (auto when piped) and `--project-dir`. Full artifact I/O in [`references/workflow-matrix.md`](../references/workflow-matrix.md).

### Bootstrap & maintenance

| Command | Purpose |
|---------|---------|
| `wh init` | Detect deps, resolve docs, fetch subscribed sources, write extraction handoff. `--detect-only` for scan-only. `--personal` / `--hooks` / `--ci` for one-time setup side-tasks. |
| `wh reinit` | Re-resolve deps; flags version AND content-hash drift. Writes `refresh-diff.json` with per-rule `re_extraction_candidates` + canned `extraction_prompt`. `--check` exits non-zero on drift for CI. |
| `wh set-sources` | Lower-level resolution-only slice of init. Normally invoked implicitly. |

### Authoring

| Command | Purpose |
|---------|---------|
| `wh extract` | Print the next worklist dep (or subscribed source) with ranked sections + quota. |
| `wh extract submit <bundle.yaml>` | Write rules as `status: candidate`. Refuses id collisions. |
| `wh approve <rule-id>` · `wh approve --all [--dep] [--confidence]` | Flip candidates to approved. |
| `wh rule add <id>` | Personal-taste shortcut. Default layer: personal (`--project` for committed). Required: `--description`, `--lang`, and either `--match <regex>` or a dotted id prefix. |
| `wh rule edit <id>` · `wh rule edit --all [--dep] [--category]` | Bump `--severity` / `--confidence` on approved rules. `--dry-run` previews. Refuses candidates. |

### Subscribing to sources

| Command | Purpose |
|---------|---------|
| `wh source add <url>` | Subscribe. Flags: `--name`, `--lang`, `--kind`, `--personal` (default), `--project`. |
| `wh source list` | Cross-layer inventory of subscribed sources. |
| `wh source remove <target>` | Unsubscribe by URL or name. Reports approved rules citing the removed URL. |
| `wh source fetch <target>` | Force re-fetch one source without a full `wh reinit`. |

### Generation

| Command | Purpose |
|---------|---------|
| `wh context` | Agent context files under `whetstone/context/`. `--terse` for one-line-per-rule bootstrap. Per-language `AGENTS.<lang>.md` sidecars emit automatically when rules span >1 language. |
| `wh tests` | Test scaffolds under `whetstone/evals/`. |
| `wh lint` | Linter overlays under `whetstone/lint/`. |
| `wh actions` | Chains context + tests + lint. Inherits `--terse` / `--lang` / `--personal`. |

### Enforcement & monitoring

| Command | Purpose |
|---------|---------|
| `wh check <path>` | Deterministic rule scan (tree-sitter + regex + lint_proxy). |
| `wh validate` | Schema + fixture validation. CI-friendly. |
| `wh status` | Both `rule_system_score` (rule health) AND `adherence_score` (code quality). Violation counts + trend in `.metrics.jsonl`. |
| `wh report` | One-page markdown: adherence + top 10 violations + drift + next actions. `--pr-comment` for PR-friendly markdown with a `<!-- whetstone-report -->` marker. `--json` for structured. |
| `wh ci` | Freshness gate with optional PR comment. `--fail-on stale \| needs_review`. |

### Inspection & self-update

| Command | Purpose |
|---------|---------|
| `wh rules query` | JIT rule lookup for agents. Filters: `--file` (infers language), `--lang`, `--dep`, `--severity`, `--personal-only`, `--project-only`, `--full`. Preferred over re-scanning `AGENTS.md` mid-turn. |
| `wh review` | Rule inspection: `[--status]`, `show <id>`, `worklist`. Read-only. |
| `wh update` | Self-update the binary from GitHub Releases. |

---

## Ship status

### 0.3.0 — tagged, shipped

The lean refactor. Surface collapsed from ~20 commands to a seven-command happy path. Deterministic `wh check` with tree-sitter + AST-scoped regex + lint-proxy. Two-layer merge (personal + project). Pre-push hook enforcing CI-parity gates locally.

### [Unreleased] — on `main`, queued for `0.4.0`

Epic 3E (Active Whetstone, closed 2026-04-20) plus post-epic follow-ups:

- **`wh rules query`** — JIT rule lookup so agents stop pre-loading `AGENTS.md`.
- **`wh context --terse` + per-language sidecars** — ~51% `AGENTS.md` size reduction on whetstone-self.
- **`wh rule add` / `wh rule edit`** — personal-taste shortcuts (skip extract/submit/approve).
- **`wh source add / list / remove / fetch`** — subscribe to custom sources (blogs, wikis, `llms.txt`, internal docs). The underlying `sources.custom[]` config was 0.3.0; the CLI surface is `[Unreleased]`.
- **`adherence_score` in `wh status`** — hybrid 60% clean-file + 40% severity-weighted. Violation trend in `.metrics.jsonl`.
- **`wh report`** — integrated one-page markdown summary.
- **Smarter `wh reinit`** — version + content-hash drift; `refresh-diff.json` now carries `re_extraction_candidates` + canned `extraction_prompt`.
- **6-gate pre-push hook** — added `ruff format --check`; matches CI exactly.
- **Format-validation tests** — snapshot tests lock required markers in all 6 context formats.

**Epic 3E acceptance deltas (all met):** session token cost −51.5%; time-to-add-personal-preference ~10 s (down from 3–5 min); single-command code-quality answer via `wh status.adherence_score`; `wh status` runtime 15.7 ms on whetstone-self.

### Near-term

| Item | Tracking |
|------|----------|
| **Cut 0.4.0** — tag, bump `Cargo.toml`, release binaries, Homebrew formula | TBD |
| **`wh patterns` reinstatement** — source is commented-out pending a clear use case | `whetstone-e2r` |
| **Config depth** — extract timeouts, resolve tuning as first-class knobs | TBD |
| **Archived-planning cleanup** — `planning/archive/` carries pre-0.3 command names | TBD |

### Future concerns (out of scope for the solo/local product)

- **Tech-debt quantification** — effort estimates, hour totals, PR debt reports. Revisit when teams need reporting. `adherence_score` + violation trend already cover most of this need for solo use.
- **Local MCP server** exposing `wh rules query` / `wh check` as MCP tools for dynamic consumption during a turn. Depends on the JIT query surface being stable in the wild first.

### Longer-term (Epic 4: Platform + Registry)

- Shared rule registry — pre-extracted, community-ranked rules for popular deps.
- Publishing — users / teams publish rulesets (`extends: @user/fastapi-strict`).
- Signal promotion (`wh evolve`) — AI verdicts graduate to deterministic signals over time.
- Whetstone as a Service — GitHub App, pooled LLM access.

Relevant only once Whetstone expands beyond solo/local. Not blocking anything today.

### Deferred in the 0.3.0 lean refactor

Removed, not deprecated. Reintroduce only if the core loop proves stable enough to add breadth.

| Removed | Why |
|---------|-----|
| `wh propose` / `wh apply` | Replaced by `wh extract submit` + `wh approve`. |
| `wh promote` / `wh layers` | 2-layer merge (personal + project) needs neither. |
| `wh bench run` / `snapshot` | Research tool; not part of the agent-coding-rules value loop. |
| `wh eval generate / run / calibrate` | Kept heavy AI round-trips out of the hot path. |
| `wh patterns` | Commented-out on disk; see `whetstone-e2r`. |
| `wh config show / validate` | Inspector deferred; config still loads. |
| Team `extends:` + built-in rules layer | Reintroduce on real multi-team demand. |
| AI eval signal strategy | Every rule now requires a deterministic signal. |

---

## Design principles

1. **High confidence or silence.** Five trusted rules beat fifty noisy ones. Every rule needs a deterministic signal and a documentation citation. Under 90% confident? Don't propose it.
2. **CLI as structured oracle.** The binary answers questions with JSON. The agent reasons between calls. `SKILL.md` teaches the workflow.
3. **The agent IS the LLM.** No API key in the binary. The user's existing agent performs extraction and judgment.
4. **Complement, don't compete.** Whetstone fills the gap that ruff / biome / clippy don't cover.
5. **Generated outputs are the product.** A teammate who never installs Whetstone still gets every rule enforced (via generated tests in CI) and every agent guided (via committed context files).
6. **Incremental by default.** Manifest fingerprinting, content hashing, cache TTL, resumable resolution. Don't redo work.
7. **Lean over comprehensive.** If a feature doesn't appear in the seven-command happy path, it belongs behind `--advanced` or gets deferred.

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
| `SKILL.md` | The agent skill — workflow loaded by Claude Code, Cursor, etc. |
| `AGENTS.md` | Universal agent instructions for this repo |
| `CLAUDE.md` | Claude Code-specific instructions |
| `CHANGELOG.md` | Release notes |
| `references/extraction-prompt.md` | The extraction prompt — core IP |
| `references/rule-schema.yaml` | Rule YAML format |
| `references/signal-strategies.md` | Signal decomposition guide |
| `references/workflow-matrix.md` | Shipped command matrix with artifact I/O |
| `references/handoff-schema.md` | `.state/*.json` contracts |
| `planning/whetstone-logic-flow.mmd` | Visual flow chart (mermaid) |
| `planning/measurements/epic-3e-baseline.md` | Token / runtime baselines + Epic 3E delta targets |
| `planning/measurements/adherence-score-design.md` | Hybrid scoring formula design |
| `scripts/measure-epic-3e.sh` | Repeatable measurement harness |
| `.githooks/pre-push` | Pre-push gate — all 6 quality gates mirror CI exactly |

---

*Whetstone sharpens the tools that write your code.*
