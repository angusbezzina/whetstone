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
| `wh init` | `deps`, `detect-deps` | detect / setup | `manifests.json`, `inventory.json` | `manifests.json`, `inventory.json` (with `--incremental`) | Default mode is dependency detection. `--personal` scaffolds `whetstone/.personal/` + gitignore. `--hooks` installs session + post-merge git hooks. `--ci --schedule=<cadence>` writes `.github/workflows/whetstone-check.yml`. |
| `wh set-sources` | `sources`, `resolve-sources` | resolve | stdin or `--input` JSON from `wh init`, `source-cache.json` | `source-cache.json` | Lower-level slice of doctor. |
| `wh validate` | `validate-rules` | — | `references/rule-schema.yaml` (or binary-embedded fallback), all layers | — | Schema + fixtures validator. CI-friendly. |
| `wh context` | `generate-context` | generate | layered rules (personal is excluded from committed output) | `whetstone/context/*` by default; `--personal` routes to `whetstone/.personal/context/` | `--personal` emits personal-only context. Committed output never leaks personal rules. |
| `wh tests` | `generate-tests` | generate | layered rules | `whetstone/evals/**`, `whetstone/lint/*` by default; `--personal` routes to `whetstone/.personal/evals/` etc. | Signals with a `match` regex produce real checks; without, tests are TODO stubs (see "Test fidelity" below). |
| `wh check` | — | monitor / enforce | layered rules, source files, linter config | — | Deterministic enforcement runner. Uses tree-sitter for `ast_query` and `ast_scope`, regex for `match:`, and linter-config verification for `lint_proxy`. Exits non-zero on violations or config gaps unless `--no-fail` is set. |
| `wh review` | — | approve | writable rules (`whetstone/rules/**`, `whetstone/.personal/rules/**`), handoff artifacts | — | Lists rules by lifecycle status, shows per-rule context, or builds a focused queue from `extraction-handoff.json` + `refresh-diff.json`. |
| `wh apply` | — | approve / lifecycle | writable rules, current layered approved ruleset | `whetstone/.state/review-log.jsonl` | Applies lifecycle transitions without hand-editing YAML. Supports approve / deny / deprecate / supersede, dry-run, and batch JSON input. |
| `wh bench run` | — | monitor / trust | benchmark corpus, `wh check` output | — | Runs the benchmark corpus and reports precision/recall/F1 per scenario. `--check` exits non-zero on regressions below `--min-f1`. |
| `wh bench snapshot` | — | monitor / trust | same as `wh bench run` | `whetstone/.state/bench-snapshot.json` | Persists the latest benchmark result as a baseline snapshot. |
| `wh layers` | — | inspect | all four layers | — | Prints a JSON summary of rule counts per layer and which layer each rule resolves to. Use this to debug merges + deny lists. |
| `wh promote` | — | lifecycle | source layer rule file | target layer rule file | `wh promote <rule-id> --to personal\|project\|team`. Monotonic — cannot promote downward. `--keep-source` copies instead of moving. |
| `wh status` | — | monitor | project rules, `whetstone/.state/*`, `whetstone/.metrics.jsonl` | `whetstone/.metrics.jsonl` (snapshot) | `--score`, `--history`, `--no-snapshot`, `--no-drift-check` are the common flags. Status today reports project-only totals; built-in/team/personal counts live in `wh layers`. |
| `wh ci` | `check`, `ci-check` | monitor (CI) | same as status | — | Exits non-zero with `--fail-on stale` or `--fail-on needs_review`. |
| `wh eval generate` | — | monitor | layered rules | `whetstone/evals/ai/*.yaml` | Only emits definitions for rules with `ai` signals or `ai_eval` config. |
| `wh eval run` | — | monitor | layered rules, `src/**` | `whetstone/.state/eval-requests.json` when AI review is needed | `--deterministic-only` skips AI requests (use in CI). `--collect` merges agent verdicts. |
| `wh eval calibrate` | — | monitor | rules with `ai_eval` | `whetstone/.state/calibration-requests.json` | `--collect` compares agent verdicts to golden examples and reports agreement rate. |
| `wh patterns` | `detect-patterns` | extract (optional) | agent transcripts (scoped), git log, GitHub PRs | `whetstone/.last-run` | Opt-in. Scoped to the current project by default; use `--global-transcripts` to widen. |
| `wh update` | — | — | — | replaces the `whetstone` binary | Self-update from GitHub Releases. Does **not** touch rules. |

> All commands accept `--json` (auto-enabled when piped). Project-scoped
> commands accept `--project-dir` (default: `.`). Human-readable progress goes
> to stderr; JSON payloads go to stdout.

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
| `ast` with `match:` | any | regex fallback with a `// TODO: upgrade to AST` note | always; structural enforcement lives in `wh check` |
| `ast` without `match:` | any | TODO stub | when extraction omitted the regex; use `wh check` for structural enforcement |
| `lint_proxy` | any | deferred to ruff/biome/clippy overlay | always |
| `ai` | any | deferred to `wh eval run` (produces an eval definition instead of a test case) | always |

> **Guidance:** every `pattern` signal SHOULD include a concrete `match` regex
> at extraction time. Without one, the generated test is a TODO stub that
> documents the rule but enforces nothing.

---

## Rule layer precedence

Whetstone resolves rules through four layers. Precedence is most-specific-wins:

```
personal > project > team > built-in
```

| Layer | Location | Committed? | Deny list |
|-------|----------|------------|-----------|
| personal | `whetstone/.personal/rules/**` | NO (`.gitignored`) | `whetstone/.personal/config.yaml` `deny:` |
| project | `whetstone/rules/**` | yes | `whetstone/whetstone.yaml` `deny:` |
| team | `extends:` clones (under `whetstone/.cache/teams/...`) or `whetstone/.team/rules/**` | team repos commit, local `.team/` is optional | team config `deny:` (future) |
| built-in | embedded in the binary (`whetstone:recommended/*`) | n/a | any layer's `deny:` excludes built-in by id |

At every level, a deny list removes a rule **from that level and every
broader level** — so a project deny silences both project-level and built-in
rules with that id, while personal deny silences everything except a
personal rule with the same id.

`whetstone/.personal/` is auto-added to `.gitignore` by `wh init --personal`.
`wh context` / `wh tests` run without flags emit committed output that
**excludes** the personal layer. Personal outputs live under
`whetstone/.personal/context/` and `whetstone/.personal/evals/` and require
the `--personal` flag.

## Extends

`whetstone.yaml extends:` references external team rulesets:

```yaml
extends:
  - whetstone:recommended            # embedded built-in — no fetch
  - github.com/acme/whetstone-rules  # cloned to whetstone/.cache/teams/acme/whetstone-rules
  - "@acme/rules"                    # future shared registry (not shipped)
```

Git-cloned repos are expected to contain `whetstone/rules/**` (project
layout) or `rules/**` (team-only publisher layout). `wh refresh` re-pulls
cached clones; otherwise the cache is reused.

## Global personal config

`~/.whetstone/config.yaml` holds user-wide defaults that merge into every
project's `WhetstoneConfig`:

```yaml
default_formats: [agents.md, .cursorrules]
deny:
  - rust.prefer-str-params   # global silence across all projects
sources:
  custom:
    - url: https://my-site.example/llms.txt
      name: My personal reference
```

Project `whetstone.yaml` wins on any field it explicitly sets; deny lists
**union** rather than override.

## Planned (not shipped)

The following are referenced in the roadmap but not yet available. Do not link
users to them as working features:

- `wh evolve` — signal promotion from AI verdicts to deterministic signals
- Tree-sitter-backed `ast` signal analysis
- MCP server
- Shared rule registry (the `@user/config` extends form is accepted by the
  parser but currently reports `not_implemented`)
- Single-file HTTP `extends:` (accepted by the parser; `not_implemented`)

See [`planning/whetstone-roadmap-v2.md`](../planning/whetstone-roadmap-v2.md) for the epic-level plan.
