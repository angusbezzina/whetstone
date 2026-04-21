# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-04-21

Epic 4A first slice: interactive TUI dashboard. The binary is now the operator's dashboard when launched from a terminal.

### Added
- **`wh tui` (Epic 4A)** — interactive TUI dashboard powered by ratatui + crossterm. Amber `#FF7E00` accent, ALL-CAPS footer, vim-first keybinds (`1`–`7` screen switch, `R` refresh, `?` help, `Q`/`Esc` quit). Renders real health data (`rule_system_score`, `adherence_score`, violation counts, drift deps, top-5 violations). Responsive: master/detail panels at ≥80×20, compact single-panel fallback below, "terminal too small" notice under 50×15. First vertical slice ships only the Dashboard + Help screens; Rules / Sources / Extract / Check / Report / Drift screens are stubbed with "Coming soon" placeholders that point at their CLI counterparts. Tracks `whetstone-tnj`.
- **Bare `wh` on a TTY launches the TUI** automatically. Piped / redirected / `--json` falls through to `--help` as before.

## [0.4.0] - 2026-04-21

Epic 3E — Active Whetstone. The goal-review on 2026-04-20 identified four architectural gaps that kept Whetstone from answering "is my code in good shape?" as well as it answered "are my rules in good shape?". This release closes those gaps: agents can query rules on demand (no more pre-loading), `wh status` returns a true code-quality score, adding a personal preference takes one command, subscribing to a blog / wiki / internal doc takes one command, and `wh reinit` surfaces per-rule re-extraction candidates instead of a flat dep list.

**Acceptance deltas from Epic 3E measurement gate (all met):** session token cost −51.5%; time-to-add-personal-preference ~10 s (from 3–5 min); single-command code-quality answer via `wh status.adherence_score`; `wh status` runtime 15.7 ms on whetstone-self.

### Added
- **`wh rules query`** — JIT rule lookup. Filters by `--file`, `--lang`, `--dep`, `--severity`, `--personal-only` / `--project-only`, with `--full` to include signals and golden examples. Agents should prefer this mid-turn over re-scanning `AGENTS.md`. First deliverable of Epic 3E (whetstone-n34), theme A.
- Generated `AGENTS.md` now carries a per-file lookup pointer directing agents at `wh rules query --file <path>`.
- `SKILL.md` documents the mid-turn lookup pattern.
- **`wh context --terse` / `wh actions --terse`** — one-line-per-rule bootstrap (~50% byte reduction on whetstone-self). Agents fall back to `wh rules query --full` for details. Closes `whetstone-ydw`.
- **Per-language sidecars** — when rules span >1 language, `wh context` / `wh actions` additionally emit `whetstone/context/AGENTS.<lang>.md` (one per language). Tools with per-language hooks can point at the narrower file. Closes `whetstone-2gw`.
- **`wh rule add <id>`** — personal-taste shortcut. Writes a rule directly to `whetstone/.personal/rules/<lang>/<dep>.yaml` as `status: approved`, bypassing the extract/submit/approve loop. Flags: `--description`, `--match` (regex signal), `--severity` / `--confidence` / `--category` / `--lang` / `--dep` / `--source-url`, `--project` to target the committed layer. Closes `whetstone-9uh`.
- **`wh rule edit <id> | --all [--dep] [--category]`** — bump `severity` or `confidence` on approved rules without hand-editing YAML. Bulk selectors via `--all`. `--dry-run` previews. Refuses candidate-status rules. Closes `whetstone-5eb`.
- **`wh status` now reports `adherence_score`** — 0–100 "is my code in good shape?" number derived from `wh check` violations, distinct from the existing `rule_system_score`. Hybrid formula: 60% clean-file ratio + 40% severity-weighted. See `planning/measurements/adherence-score-design.md`. Closes `whetstone-0m0`, `whetstone-90m`, `whetstone-m3k`.
- **Violation-trend snapshots** — `.metrics.jsonl` now records `adherence_score` + `violation_counts` per snapshot so `wh status --history` can show deltas over time. Closes `whetstone-m2q`.
- **`wh report`** — integrated one-page summary composing rule-system + adherence scores, top 10 violations (ranked by severity), drift, and next actions. `--pr-comment` emits PR-friendly markdown with a `<!-- whetstone-report -->` tracking marker. `--json` for structured output. Closes `whetstone-hpq`.
- **Refresh-diff carries `re_extraction_candidates`** — `wh reinit` now emits a per-rule list of approved rules citing drifted deps, with `drift_types` (`version` and/or `content_hash`), current severity, and current source URL. Non-JSON reinit output surfaces up to 10 candidates so the agent can see what needs attention at a glance. Closes `whetstone-awj`.
- **Canned re-extraction prompt in refresh-diff** — `extraction_prompt` field in `refresh-diff.json` names the affected rules and tells the agent the exact sequence to use (`wh rule edit` / delete / `wh extract submit`). Closes `whetstone-jrs`.
- **Content-hash drift detection in `wh reinit`** — deps whose manifest version hasn't bumped but whose documentation content has now appear in `refresh-diff.json` with `drift_type: content_hash`. Enabled by default; no flag needed since `wh reinit` already re-fetches all deps. Closes `whetstone-nuh`.
- **`wh source add / list / remove / fetch`** — CLI surface for custom rule sources. Previously the underlying `sources.custom[]` field existed in `whetstone.yaml` and `whetstone/.personal/config.yaml` but had no UX — users had to hand-author YAML. Now: `wh source add https://blog.example.com/py --name py-tips --lang python --kind blog` subscribes (personal by default, `--project` for committed). `wh source remove` reports any approved rules citing the removed URL. `wh source fetch <name>` force re-fetches a single source. `wh source list` shows both layers. Closes `whetstone-gpe`.
- Baseline measurement harness at `scripts/measure-epic-3e.sh` and log at `planning/measurements/epic-3e-baseline.md` (closes `whetstone-piy`).
- **6-gate pre-push hook** — `.githooks/pre-push` now also runs `ruff format --check` (step 4 of 6), matching CI exactly. Previously the hook ran only `ruff check` which missed formatting drift. Install via `git config core.hooksPath .githooks && chmod +x .githooks/pre-push`.
- **Format-validation tests** (`whetstone-2r9`) — snapshot tests lock the minimum required markers in all 6 context formats (agents.md, claude.md, .cursorrules, copilot-instructions.md, .windsurfrules, codex.md) so a silent tool-parser divergence is caught pre-push.

### Fixed
- Personal-only projects (created via `wh rule add --personal` without explicit `wh init`) now score and scan correctly. Previously the "is the project initialized?" gate in `wh status` / `wh check` / `wh context` only looked for `whetstone.yaml` and missed the `.personal/rules/` directory. New shared `crate::layers::project_is_initialized` helper handles both.
- Generated Python eval scaffolds no longer emit unused `import re` (emitted only when a rule signal has a `match:` pattern) or unused `import glob` in conftest. Tera's trailing-blank-line artifacts are stripped before writing so `ruff format --check` stays green on regenerated fixtures.

## [0.3.0] - 2026-04-20

Lean refactor. Seven-command happy path:

```
wh init  →  wh extract  →  wh extract submit  →  wh approve --all
                                                        │
                                                        ▼
                                                   wh actions
                                                        │
                                                        ▼
                                                  wh check src/
                                                        │
                                                        ▼
                                                   wh reinit
```

### Added
- `wh extract` — walk the extraction worklist interactively.
- `wh extract submit <bundle.yaml>` — write candidate rules to
  `whetstone/rules/<lang>/<dep>.yaml`, failing on any id collision.
- `wh approve <rule-id>` and `wh approve --all [--dep] [--confidence]` —
  flip candidate rules to approved with batch selectors.
- `wh lint` — emit ruff / biome / clippy overlays from `lint_proxy`
  signals. Split out of `wh tests`.
- `wh actions` — chain context + tests + lint in one command.

### Changed
- `wh tests` no longer writes lint configs; use `wh lint` instead.
  The `lint_configs` key is removed from its output.
- `wh refresh` is now `wh reinit` (pairs with `wh init`).
- Rule status lifecycle reduced to `candidate` and `approved`.

### Removed — command aliases
All historical command aliases have been dropped in favor of a single
canonical name per verb. If your scripts use any of these, update them:

| Removed | Use instead |
|---------|-------------|
| `wh doctor`, `wh start`, `wh deps`, `wh detect-deps` | `wh init` |
| `wh refresh`, `wh refresh-rules` | `wh reinit` |
| `wh gen` | `wh actions` |
| `wh sources`, `wh resolve-sources` | `wh set-sources` |
| `wh generate-context` | `wh context` |
| `wh generate-tests` | `wh tests` |
| `wh validate-rules` | `wh validate` |
| `wh ci-check` | `wh ci` |
- Layer merge collapsed to personal + project only.

### Removed
- Commands: `wh propose`, `wh apply`, `wh bench`, `wh eval`,
  `wh promote`, `wh layers`, `wh config`, `wh patterns` (the patterns
  module is commented out in source; the rest were deleted).
- Rule review subcommands: `wh review queue`, `wh review diff`.
- Rule schema fields: `risk`, `linter_gap`, `source_kind`, `proposed_at`,
  `proposed_by`, `approved_at`, `denied_reason`, `deprecated_reason`,
  `superseded_by`, `ai_eval`.
- Signal strategy `ai`; every rule must have `ast`, `pattern`, or
  `lint_proxy` signals.
- Built-in rules (the `src/builtin/` directory and the built-in layer in
  the merge).
- Team layer + `extends:` config key.
- `bench` and `extends` config keys.
- `whetstone/.state/eval-requests.json`, `eval-verdicts.json`,
  `calibration-requests.json`, `calibration-verdicts.json` writers.
- `references/proposal-schema.md`.

## [Unreleased]

### Added
- **Tera template engine** — all agent context files, eval tests, and linter
  overlays now render from `.tera` templates embedded in the binary. Language
  escape filters (`re_escape_py`, `re_escape_ts`, `re_escape_rust`) keep
  target-language quoting correct without hand-rolled string concatenation.
- **Tree-sitter substrate** — `src/ast/` ships parsers for Python, TypeScript
  (TSX), and Rust, with a thread-local parser cache and query helpers for
  functions, classes, imports, decorators, and attributes.
- **`wh check`** — scan source files against approved rule signals. Supports
  `ast` signals with raw tree-sitter queries (`ast_query:`), `pattern`
  signals with AST-scoped regex (`ast_scope:`), and a `lint_proxy` verifier
  that reads `ruff.toml`/`pyproject.toml`/`biome.json` to confirm the mapped
  rule is enabled in the project's lint config. Output includes a
  `config_issues` list so linter gaps are actionable.
- **`wh review` and `wh apply`** — first-class lifecycle CLI for candidate,
  approved, denied, deprecated, and superseded rules. Monotonic transitions,
  required reasons on deny/deprecate/supersede, cross-checked
  `--superseded-by` targets, dry-run and batch apply, and a concurrency-safe
  audit log at `whetstone/.state/review-log.jsonl`. YAML mutations use a
  line-based editor so authored comments and formatting survive.
- **`wh review queue`** — actionable queue built from
  `extraction-handoff.json` + `refresh-diff.json` so refresh runs flow
  directly into review work.
- **`wh bench`** — benchmark harness with per-scenario precision/recall/F1.
  Supports deterministic, layered, and eval scenarios; scenarios can declare
  a scenario-local project_dir (`project_dir: .` in `meta.yaml`) so layered
  rule resolution can be exercised in isolation. `--check --min-f1` gates CI
  on regressions; the `bench-corpus` GitHub workflow job enforces F1=1.0 on
  the shipped corpus.

### Changed
- **Rule schema** — optional `ast_query` and `ast_scope` fields on `signals`.
  `ast_query` is a raw tree-sitter S-expression query (runs against the
  matched language); `ast_scope` scopes a pattern regex to a specific AST
  node kind. Existing rules continue to work unchanged.
- **Built-in rules upgraded to tree-sitter** — every built-in where a
  syntactic check is stricter than the regex now ships an `ast_query` or
  `ast_scope`: Python `no-shell-true`, `mutable-default-arguments`,
  `no-except-pass`, `no-requests-without-timeout`, `open-without-encoding`;
  Rust `expect-over-unwrap`, `timeout-on-http-clients`, `error-context`,
  `prefer-str-params`; TypeScript `no-any`, `no-var`, `no-non-null-assertion`.
  The original `match:` regex is retained as a fallback so test generation
  (`wh tests`) and grammar-failure paths still enforce the rule.
- **`wh check` falls back to regex on tree-parse failure** — when a rule
  has both `ast_query` and `match:` but the grammar fails to parse a file,
  the regex fires instead of silently skipping the rule.

### Removed
- **BREAKING**: `wh ci check` alias — `check` is now a top-level command
  (`wh check`). Existing CI workflows that call `wh ci check` should switch
  to plain `wh ci` for freshness checks, or `wh check` for rule scanning.

## [0.2.0] - 2026-04-12

### Added
- **Multi-tier content fetching** — 4-tier resolve pipeline: llms.txt →
  registry README (npm/PyPI/crates.io) → HTML docs conversion → GitHub
  changelog. All dependencies now get content (previously null for non-llms.txt).
- **Changelog discovery** — probes GitHub repos for CHANGELOG.md, filters
  to last 18 months, includes as a separate `sections` entry alongside README.
- **Sections array** — resolver output now includes labeled sections (readme,
  changelog, llms_txt) for per-section extraction.
- **Custom source support** — `sources.custom` in `whetstone.yaml` lets
  users add arbitrary URLs (blogs, team guides, any public page).
- **Built-in rules** — 5 Rust rules ship embedded in the binary
  (`whetstone:recommended`). Project rules override by ID. Deny list support.
- **`match` field on signals** — concrete regex patterns that enable real
  test generation instead of TODO stubs.
- **Real regex test generation** — generated tests scan source files with
  actual regex checks, reporting violations with file path and line number.
- **`wh refresh` command** — detect drift and re-resolve changed deps.
  `--check` flag for CI exits non-zero on drift.
- **Source attribution** — `content_origin` (how binary fetched it) and
  `source_kind` (official_docs, changelog, blog, social, etc.) fields.
- **`wh validate` checks real rules** — now validates `whetstone/rules/`
  in addition to test fixtures.

### Changed
- **SKILL.md rewritten** — teaches agents the sections/changelog/source_kind
  model, match patterns for signals, and the full extraction workflow.
- **README.md** — comparison table (vs Semgrep, Continue.dev, CodeRabbit),
  worked example showing full extraction flow, updated capabilities section.
- **Extraction prompt** — multi-section content guidance, source_kind
  attribution requirement, match pattern documentation.

### Fixed
- `wh validate` now checks `whetstone/rules/` (was only checking test fixtures).

## [0.1.2] - 2026-04-05

### Added
- **`wh update`** — self-update command that downloads the latest release
  binary from GitHub, verifies sha256 checksum, and replaces the running
  binary atomically. Flags: `--check` (just check), `--force` (reinstall).
- **`wh` binary alias** — short name for `whetstone`, installed alongside
  the main binary.
- **Progress bar** during dependency resolution via indicatif.
- **Human-friendly default output** — all commands now print readable text
  by default. Use `--json` (global flag) for machine-readable JSON.
  Auto-detects piped stdout.
- **Scoped package grouping** — `@radix-ui/*` and similar npm scopes shown
  as a single grouped line in human output; JSON gains a `scope` field.

### Changed
- Command renames (old names kept as hidden aliases):
  `detect-deps`→`init`, `resolve-sources`→`set-sources`,
  `generate-context`→`context`, `generate-tests`→`tests`,
  `validate-rules`→`validate`, `detect-patterns`→`patterns`,
  `ci-check`→`ci`. Doctor gains visible alias `start`.

### Fixed
- Box-drawing characters now consistent (no mixed ASCII `=` and Unicode `═`).
- `status` no longer prints "Monorepo detected" twice.

## [0.1.1] - 2026-04-05

### Fixed
- Switch reqwest from native-tls to rustls-tls so cross-compilation for
  aarch64-unknown-linux-gnu no longer requires a system OpenSSL. The binary
  is now fully self-contained on all targets.
- Update macOS x86_64 CI runner from deprecated macos-13 to macos-14.

## [0.1.0] - 2026-04-05

First public release. Whetstone is a single self-contained Rust binary with
no Python runtime dependency.

### Added
- **Dependency detection** (`detect-deps`) for Python, TypeScript, and Rust,
  including monorepo support and incremental fingerprinting.
- **Documentation resolution** (`resolve-sources`) via PyPI, npm, and
  crates.io registry APIs with llms.txt probing and content hashing.
- **One-command bootstrap** (`doctor`) from zero to working rules.
- **Health monitoring** (`status`) with drift detection, freshness scoring,
  dimensional breakdown, and append-only metric snapshots.
- **CI integration** (`ci-check`) with JSON output, PR comment generation,
  and configurable `--fail-on` thresholds.
- **Agent context generation** (`generate-context`) for CLAUDE.md, AGENTS.md,
  .cursorrules, and three other formats.
- **Test generation** (`generate-tests`) producing pytest, vitest, and cargo
  test scaffolds plus ruff, biome, and clippy lint overlays.
- **Pattern mining** (`detect-patterns`) from agent transcripts, git history,
  and GitHub PR review comments. Project-scoped by default for privacy.
- **Rule schema validation** (`validate-rules`) replacing the legacy Python
  validator with identical output.
- **GitHub Action** (`action.yml`) for CI freshness gating with PR comments.
- **Install script** (`install.sh`) with sha256 checksum verification,
  platform detection, and `--version` pinning.
- **Homebrew formula template** (`packaging/homebrew/whetstone.rb`).
- **Release workflow** building Linux and macOS binaries for x86_64 and
  aarch64 with cross-compilation support.

[0.5.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.5.0
[0.4.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.4.0
[0.3.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.3.0
[0.1.2]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.2
[0.1.1]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.1
[0.1.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.0
