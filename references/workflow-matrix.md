# Whetstone Workflow Matrix

> Single source of truth for shipped commands, their lifecycle role, and the
> artifacts they read or write. Keep this in sync with `src/cli.rs` and
> `CHANGELOG.md`.

This matrix is load-bearing for the README, SKILL.md, and the roadmap. When a
new command ships (or an existing command changes its handoff artifacts), update
this file first and cross-link from the other docs.

---

## Lifecycle stages

Whetstone's core loop has six stages. Each command below maps to one or more
stages:

1. **Detect** — find dependencies from manifests
2. **Resolve** — fetch documentation content for each dep
3. **Extract** — agent reads docs, proposes candidate rules
4. **Approve** — user reviews candidates (candidate → approved/denied)
5. **Generate** — emit tests, lint overlays, agent context from approved rules
6. **Monitor** — health score, drift detection, eval runs, refresh

---

## Command matrix

| Command | Aliases | Stages | Reads (state) | Writes (state) | Notes |
|---------|---------|--------|---------------|----------------|-------|
| `wh doctor` | `start` | detect + resolve + hand off | `manifests.json`, `inventory.json`, `source-cache.json` | `extraction-handoff.json`, cache, inventory, manifests | One-command bootstrap. Writes `whetstone/.state/extraction-handoff.json` describing what the agent should extract next. |
| `wh refresh` | `refresh-rules` | detect + resolve + hand off (changed-only) | same as doctor | `refresh-diff.json`, cache, inventory | `wh refresh --check` exits non-zero if drift was detected — wire this into CI. |
| `wh init` | `deps`, `detect-deps` | detect | `manifests.json`, `inventory.json` | `manifests.json`, `inventory.json` (with `--incremental`) | Lower-level slice of doctor. |
| `wh set-sources` | `sources`, `resolve-sources` | resolve | stdin or `--input` JSON from `wh init`, `source-cache.json` | `source-cache.json` | Lower-level slice of doctor. |
| `wh validate` | `validate-rules` | — | `references/rule-schema.yaml`, `whetstone/rules/**`, `tests/fixtures/**` | — | Schema + fixtures validator. CI-friendly. |
| `wh context` | `generate-context` | generate | `whetstone/rules/**`, built-in rules | `whetstone/context/*` and equivalents | Built-in rules merge unless `whetstone.yaml` denies them. |
| `wh tests` | `generate-tests` | generate | `whetstone/rules/**`, built-in rules | `whetstone/evals/**`, `whetstone/lint/*` | Signals with a `match` regex produce real checks; without, tests are TODO stubs (see "Test fidelity" below). |
| `wh status` | — | monitor | `whetstone/rules/**`, `whetstone/.state/*`, `whetstone/.metrics.jsonl` | `whetstone/.metrics.jsonl` (snapshot) | `--score`, `--history`, `--no-snapshot`, `--no-drift-check` are the common flags. |
| `wh ci` | `check`, `ci-check` | monitor (CI) | same as status | — | Exits non-zero with `--fail-on stale` or `--fail-on needs_review`. |
| `wh eval generate` | — | monitor | `whetstone/rules/**`, built-in rules | `whetstone/evals/ai/*.yaml` | Only emits definitions for rules with `ai` signals or `ai_eval` config. |
| `wh eval run` | — | monitor | `whetstone/rules/**`, `src/**` | `whetstone/.state/eval-requests.json` when AI review is needed | `--deterministic-only` skips AI requests (use in CI). `--collect` merges agent verdicts. |
| `wh eval calibrate` | — | monitor | rules with `ai_eval` | `whetstone/.state/calibration-requests.json` | `--collect` compares agent verdicts to golden examples and reports agreement rate. |
| `wh patterns` | `detect-patterns` | extract (optional) | agent transcripts (scoped), git log, GitHub PRs | `whetstone/.last-run` | Opt-in. Scoped to the current project by default; use `--global-transcripts` to widen. |
| `wh update` | — | — | — | replaces the `whetstone` binary | Self-update from GitHub Releases. Does **not** touch rules. |

> All commands accept `--json` (auto-enabled when piped) and `--project-dir`
> (default: `.`). Human-readable progress goes to stderr; JSON payloads go to
> stdout.

---

## Handoff artifacts

Whetstone uses file-based handoffs to keep the binary deterministic and the
agent free to do LLM work. The on-disk schemas are documented in
[`handoff-schema.md`](handoff-schema.md).

| Artifact | Written by | Read by | Schema |
|----------|------------|---------|--------|
| `whetstone/.state/extraction-handoff.json` | `wh doctor` / `wh refresh` | agent (extraction), user (review) | `handoff-schema.md` §extraction-handoff |
| `whetstone/.state/refresh-diff.json` | `wh refresh` | agent (focused re-extraction), user (review) | `handoff-schema.md` §refresh-diff |
| `whetstone/.state/eval-requests.json` | `wh eval run` | agent (judgment) | `handoff-schema.md` §eval-requests |
| `whetstone/.state/eval-verdicts.json` | agent | `wh eval run --collect` | `handoff-schema.md` §eval-verdicts |
| `whetstone/.state/calibration-requests.json` | `wh eval calibrate` | agent (judgment) | same shape as eval-requests |
| `whetstone/.state/calibration-verdicts.json` | agent | `wh eval calibrate --collect` | same shape as eval-verdicts |

---

## Rule lifecycle statuses

Rules in `whetstone/rules/**/*.yaml` carry an explicit `status` field:

| Status | Meaning | Next transition |
|--------|---------|-----------------|
| `candidate` | Proposed by the extraction agent, not yet reviewed | user approves (→ `approved`) or denies (→ `denied`) |
| `approved` | Reviewed and accepted; counted in generation | may become `deprecated` after a refresh |
| `denied` | Reviewed and rejected; persisted for audit | terminal unless explicitly revived |
| `deprecated` | Previously approved; superseded or no longer backed by docs | terminal; replaced by a new rule via `superseded_by` |

The `approved: true/false` boolean stays for backward compatibility with the
generator code paths, but the agent SHOULD also set `status` explicitly during
approval. See [`rule-schema.yaml`](rule-schema.yaml) for the full field list.

---

## Test fidelity

`wh tests` emits native test files that check rules against the project source.
Fidelity depends on the signal type:

| Signal | Language | Check written | When |
|--------|----------|---------------|------|
| `pattern` with `match:` | Python | real `re.search` loop | always |
| `pattern` with `match:` | TypeScript | real `RegExp.test` loop | always |
| `pattern` with `match:` | Rust | real `regex::Regex::new` loop | always |
| `pattern` without `match:` | any | TODO stub | when extraction omitted the regex |
| `ast` with `match:` | any | regex fallback with a `// TODO: upgrade to AST` note | always (tree-sitter not yet wired in) |
| `ast` without `match:` | any | TODO stub | when extraction omitted the regex |
| `lint_proxy` | any | deferred to ruff/biome/clippy overlay | always |
| `ai` | any | deferred to `wh eval run` (produces an eval definition instead of a test case) | always |

> **Guidance:** every `pattern` signal SHOULD include a concrete `match` regex
> at extraction time. Without one, the generated test is a TODO stub that
> documents the rule but enforces nothing.

---

## Planned (not shipped)

The following are referenced in the roadmap but not yet available. Do not link
users to them as working features:

- `wh promote` — move rules between personal/project/team layers
- `wh evolve` — signal promotion from AI verdicts to deterministic signals
- Tree-sitter-backed `ast` signal analysis
- MCP server
- Shared rule registry

See [`planning/whetstone-roadmap-v2.md`](../planning/whetstone-roadmap-v2.md) for the epic-level plan.
