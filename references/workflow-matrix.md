# Whetstone Workflow Matrix

> Single source of truth for shipped commands, their lifecycle role, and the
> artifacts they read or write. Keep this in sync with `src/cli.rs` and
> `CHANGELOG.md`.

This matrix is load-bearing for the README, SKILL.md, and the roadmap. When a
new command ships (or an existing command changes its handoff artifacts), update
this file first and cross-link from the other docs.

---

## Lifecycle stages

Whetstone's loop has five stages. Each command below maps to one or more
stages:

1. **Detect + Resolve** — find dependencies from manifests and fetch their docs
2. **Extract** — agent reads docs, drafts a bundle, submits as candidate rules
3. **Approve** — user flips candidates to approved (single id or batch)
4. **Generate** — emit context, tests, and lint configs from approved rules
5. **Monitor** — health score, drift detection, refresh

---

## Command matrix

There are no command aliases as of 0.3.0 — each verb has exactly one name.

| Command | Stages | Reads (state) | Writes (state) | Notes |
|---------|--------|---------------|----------------|-------|
| `wh init` | detect + resolve + hand off | `manifests.json`, `inventory.json`, `source-cache.json` | `extraction-handoff.json`, cache, inventory, manifests | Default: full bootstrap. `--detect-only` scans manifests. `--personal` scaffolds `whetstone/.personal/`. `--hooks` installs git hooks. `--ci --schedule=<cadence>` writes the CI workflow. |
| `wh reinit` | refresh | same as init | `refresh-diff.json` (with `re_extraction_candidates` + canned `extraction_prompt`), cache, inventory | `wh reinit --check` exits non-zero on drift. Flags both manifest-version drift AND content-hash drift (when stored rule hash differs from current doc hash). Wire into CI. |
| `wh set-sources` | resolve | stdin or `--input` JSON, `source-cache.json` | `source-cache.json` | Lower-level slice of init. |
| `wh extract` | extract | `extraction-handoff.json` | — | Default mode renders the dependency worklist (ranked sources, quota, next step). |
| `wh extract submit <bundle.yaml>` | extract | bundle file | `whetstone/rules/<lang>/<dep>.yaml` with `status: candidate` | Refuses to overwrite an existing file or collide on any rule id. |
| `wh approve <rule-id>` | approve | `whetstone/rules/**`, `whetstone/.personal/rules/**` | same file, mutated in place | Flips status to `approved` and `approved: true`. |
| `wh approve --all [--dep <name>] [--confidence <level>]` | approve | project rules | same files | Batch flip every matching candidate. |
| `wh actions` | generate | approved rules | everything under `whetstone/context/`, `whetstone/evals/`, `whetstone/lint/` | Chains context + tests + lint. |
| `wh context` | generate | approved rules | `whetstone/context/*` or `whetstone/.personal/context/*` | `--terse` emits a one-line-per-rule bootstrap. Per-language `AGENTS.<lang>.md` sidecars are emitted automatically when rules span >1 language. |
| `wh tests` | generate | approved rules | `whetstone/evals/**` or `whetstone/.personal/evals/**` | Signals with a `match` regex produce real checks; without, tests are TODO stubs. |
| `wh lint` | generate | approved rules | `whetstone/lint/*` or `whetstone/.personal/lint/*` | Emits `ruff.whetstone.toml`, `biome.whetstone.json`, `clippy.whetstone.toml`. |
| `wh check` | monitor / enforce | approved rules, source files | — | Deterministic enforcement: tree-sitter for `ast_query` / `ast_scope`, regex for `match:`, lint-config verification for `lint_proxy`. Non-zero exit on violations or config gaps unless `--no-fail`. |
| `wh rules query` | inspect (JIT) | approved rules | — | JIT rule lookup for agents. Filters: `--file <path>` (infers language), `--lang`, `--dep`, `--severity`, `--personal-only`, `--project-only`, `--full`. Preferred over re-reading `AGENTS.md` mid-turn. |
| `wh rule add <id>` | author | — | one rule appended to (or creates) `whetstone/(.personal/)?rules/<lang>/<dep>.yaml` as `status: approved` | Personal-taste shortcut. Required: `--description`, `--lang`, either `--match` or a dep prefix in the id. `--project` to target the committed layer. |
| `wh rule edit <id> \| --all [--dep] [--category]` | author | approved rule files | same file, severity/confidence mutated | Bump severity (`should → must`) or confidence without hand-editing YAML. `--dry-run` to preview. Refuses candidates. |
| `wh source add <url>` | author (sources) | — | `whetstone/.personal/config.yaml` (default) or `whetstone/whetstone.yaml` (`--project`) | Subscribe to a blog / wiki / llms.txt / internal doc. Appears in the extraction worklist alongside detected deps. Flags: `--name`, `--lang`, `--kind`. Refuses duplicates. |
| `wh source list` | inspect | both config layers | — | Show subscribed custom sources across personal + project layers. |
| `wh source remove <target>` | author (sources) | same configs | same configs, entry removed | Unsubscribe by URL or name. Reports any approved rules that cite the removed URL. |
| `wh source fetch <target>` | resolve | subscribed config | `source-cache.json` | Force re-fetch a single custom source without running full `wh reinit`. |
| `wh review [show <id> \| worklist]` | inspect | writable rules, handoff artifacts | — | Lists rules by lifecycle status, shows full per-rule context, or renders the extraction worklist. |
| `wh validate` | — | `references/rule-schema.yaml` (or embedded fallback), all rule files | — | Schema + fixtures validator. CI-friendly. |
| `wh status` | monitor | project rules, state files, metrics, source files for `wh check` | `whetstone/.metrics.jsonl` (snapshot w/ `adherence_score` + `violation_counts`) | Returns both `rule_system_score` (rule health) and `adherence_score` (code quality, hybrid formula). `--score`, `--history`, `--no-snapshot`, `--no-drift-check`. |
| `wh report` | monitor | project rules, source files, `refresh-diff.json` | — | One-page markdown summary: rule-system + adherence scores, top 10 violations with file/line, drift, next actions. `--pr-comment` emits the PR-friendly flavor with a `<!-- whetstone-report -->` marker. `--json` for structured output. |
| `wh ci` | monitor (CI) | same as status | — | `--fail-on stale` or `--fail-on needs_review` gates PRs. |
| `wh update` | — | — | replaces the binary | Self-update from GitHub Releases. Never touches rules. |
| `wh tui` (or bare `wh` on TTY) | inspect / navigate | project rules, `.state/*`, `.metrics.jsonl` | — | Interactive dashboard (Epic 4A). `1`–`7` switch screens, `R` refresh, `?` help, `Q` quit. Current shipped screens: Dashboard + Help; Rules / Sources / Extract / Check / Report / Drift are stubbed. |

> All commands accept `--json` (auto-enabled when piped). Project-scoped
> commands accept `--project-dir` (default: `.`). Human-readable progress goes
> to stderr; JSON payloads go to stdout.

---

## Handoff artifacts

| Artifact | Writer | Reader | Purpose |
|----------|--------|--------|---------|
| `whetstone/.state/extraction-handoff.json` | `wh init`, `wh reinit` | `wh extract`, `wh review worklist` | Worklist + ranked sources per dep |
| `whetstone/.state/refresh-diff.json` | `wh reinit` | `wh review worklist` | Drift diff for stale rules |
| `whetstone/.state/manifests.json` | `wh init`, `wh reinit` | `wh init --incremental` | Manifest fingerprints |
| `whetstone/.state/inventory.json` | same | `wh init`, `wh set-sources` | Last-seen deps |
| `whetstone/.state/source-cache.json` | `wh set-sources` | same | Content cache + hashes |
| `whetstone/.metrics.jsonl` | `wh status` | `wh status --history` | Append-only score snapshots |

---

## Deferred

Features intentionally removed in 0.3.0:

- `wh promote`, `wh layers` — the four-layer merge collapsed to personal +
  project, so promotion and per-layer inspection are no longer needed.
- `wh propose`, `wh apply`, `wh review queue`, `wh review diff` — replaced by
  the simpler `wh extract submit` + `wh approve` pair.
- `wh bench`, `wh eval`, `wh patterns` — trust/AI-eval/pattern-mining work is
  parked until the core loop stabilizes.
- `wh config` — config still works at the YAML level; the inspector UI is
  deferred.
- Built-in rules and team extends — gone. The personal layer carries
  local-only overrides; everything else lives in `whetstone/rules/`.
