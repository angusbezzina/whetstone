# Whetstone

> Sharpen the tools that write your code.

Whetstone is a **rule-intelligence layer** that derives coding rules from the documentation of your actual dependencies. It decomposes rules into deterministic checks and generates native tests, lint configs, and agent context files — all from the same approved ruleset.

Other tools execute checks, review pull requests, or apply fixes. Whetstone decides **which rules are worth enforcing** in the first place, why they matter, and how they map to deterministic enforcement and agent guidance.

Whetstone is **skill-first with a thin deterministic CLI**. The agent skill is the front door: it owns judgment — reading docs, proposing high-confidence rules, and carrying taste/type-aware guidance that can't be deterministically enforced. The CLI is the deterministic substrate the skill calls (detect, resolve + content-hash, validate, generate, scan, drift gate). See [`planning/skill-cli-boundary.md`](planning/skill-cli-boundary.md) for the authoritative split.

It's a codegen tool, not a runtime dependency. A teammate who never installs Whetstone still gets every rule enforced through standard CI and every agent guided by current instructions.

### Two loops that keep agents on standard

Whetstone closes two loops so your agents write code you consider best-practice:

1. **In-session enforcement.** A Claude Code PostToolUse hook scans each file the
   agent edits and feeds any rule violation back into the *same turn*, so the agent
   fixes it before moving on — not post-hoc in CI. Wire it in one command:
   `wh init --claude`.
2. **Taste capture.** When you express a standing preference ("never do that
   again"), the agent codifies it — a deterministic rule (verified with `wh eval`)
   or, when it needs judgment, a guidance entry — and lands it in a **personal taste
   pack** imported into every repo. Your standards, versioned once, enforced
   everywhere.

(Scope discipline — stopping an agent wandering off-task — is a different problem
and deliberately **out of scope**; Whetstone keeps agents *on standard*, not *on
scope*. See [`planning/skill-cli-boundary.md`](planning/skill-cli-boundary.md) §10.)

## Why Whetstone?

**Rules go stale.** Linter configs and coding conventions are written once at project setup. Dependencies ship new versions, deprecate APIs, and introduce better patterns. Nobody updates the rules. Agents keep writing code against outdated practices.

**Dependency-specific best practices are unenforced.** Standard linters catch syntax and formatting. They don't know that FastAPI docs recommend `async def` for route handlers, or that Pydantic deprecated `.schema()` in favor of `model_json_schema()`. These are the rules that matter most — and nothing catches them.

**Agents aren't told what they need to know.** `AGENTS.md` and `.cursorrules` are written once by hand — if they're written at all — and never updated when dependencies evolve.

Whetstone solves all three. It treats documentation as a living source of truth, converts it into enforceable checks, and keeps everything current as your dependencies evolve.

### High confidence or silence

5 rules you trust completely beats 50 you have to review. Whetstone only proposes rules backed by specific documentation with deterministic signals. A project with 40 dependencies might get rules for 8 of them — those are the 8 that have something worth enforcing.

### How Whetstone compares

| Tool | What it does | Whetstone's angle |
|------|-------------|-------------------|
| **Semgrep / CodeQL** | Custom SAST rules you write manually | Whetstone derives rules from docs — you review, not author |
| **Continue.dev** | AI code review from hand-written markdown rules | Whetstone generates the rules from dependency documentation |
| **CodeRabbit** | AI PR review (2M+ repos) | Reads Whetstone's output — `.cursorrules`, `CLAUDE.md`, `AGENTS.md` |
| **Ruff / Biome / Clippy** | Language-level linting | Whetstone catches dependency-specific rules they don't cover |

Whetstone is not a general AI code reviewer or a replacement for linters. It's the **rule-intelligence layer** — it decides which rules are worth enforcing, proves them with documentation, and generates enforcement artifacts for the tools you already use.

## Quick Start

**Prerequisites:** Rust toolchain (for building from source), or download a release binary. Git and internet access for registry lookups.

### Install

The recommended install path is `install.sh`, which downloads the latest
release binary for your platform and verifies its sha256 against the
published checksum file:

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
```

By default the binary is placed at `~/.local/bin/whetstone`. Override with
`INSTALL_DIR=/usr/local/bin` or similar. No repo checkout or Rust toolchain
is required on the target machine.

Alternatives:

```bash
# Homebrew (once the tap is published — see packaging/homebrew/README.md)
brew install angusbezzina/tap/whetstone

# From source with Cargo
cargo install --git https://github.com/angusbezzina/whetstone

# From a local checkout
cargo build --release && ./target/release/whetstone --help
```

`whetstone` is a single self-contained binary; once installed, `whetstone
init --project-dir <your-repo>` works from any directory — there is no
requirement to run it from inside the Whetstone checkout.

### Get value in one command — or one screen

Whetstone has two front doors that produce the **same** artifacts (packs,
`extends`, context, hooks, `.mcp.json`) — pick whichever fits:

- **Human:** run `whetstone` (bare, on a TTY) on a fresh project and follow the
  **Setup wizard** — Express accepts the starter packs that match your
  dependencies, or Curated lets you browse packs, preview each one's hits against
  your code *before* importing, review every rule with its citation, and see the
  first scan. Nothing is enforced until you confirm.
- **Agent:** one command wires it headlessly for a Claude Code agent:

```bash
whetstone init --claude
```

The two doors differ only in the consent moment: the wizard reviews before it
writes; `init --claude` imports pre-verified rules and tells you to review them
anytime (run `whetstone`). Same state, either way.

That detects your dependencies, imports the matching pre-verified starter packs
(FastAPI, Pydantic, SQLAlchemy, httpx, React, Next, Zod, Express, axum, serde),
generates agent context, registers the [MCP server](#use-with-coding-agents-mcp),
and installs the in-session enforcement hook. Restart your session and edit a
file — any rule violation is fed back to the agent in the same turn.

Prefer to wire it by hand, or not using Claude Code? Import a pack directly:

```bash
mkdir -p whetstone/packs
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/packs/python/fastapi.yaml \
  -o whetstone/packs/fastapi.yaml
cat > whetstone/whetstone.yaml <<'YAML'
version: 1
extends:
  - scope: project
    ref: path:./whetstone/packs/fastapi.yaml
YAML

whetstone scan src/          # flag violations now
whetstone actions lint       # emit native ruff/biome/clippy config to enforce in CI
whetstone actions context    # write CLAUDE.md / AGENTS.md so your agent follows the rules
```

Every pack rule is doc-cited, deterministic, and passes `whetstone eval`. To derive
your own rules from your dependencies' docs, use the full workflow below; to encode
your own taste, see [your personal taste pack](packs/README.md).

### Recommended repo setup for contributors

Enable the repo-managed pre-push hook so local pushes run the same quality gates used in CI:

```bash
git config core.hooksPath .githooks
chmod +x .githooks/pre-push
```

If your local Beads state gets out of sync or a new machine cannot see current
issues, repair/hydrate the local Beads Dolt database with:

```bash
./scripts/beads-repair.sh --role contributor
```

### Usage

```bash
# 1. Bootstrap — one command from zero to working extraction handoff
wh init
# → detects dependencies from pyproject.toml / package.json / Cargo.toml
# → resolves documentation URLs from registries, probes for llms.txt
# → writes whetstone/.state/extraction-handoff.json

# 2. Walk the worklist and draft candidate rules
wh extract
# The agent picks the top `ready_now` dep and authors a bundle.

# 3. Submit the candidate bundle
wh extract submit path/to/bundle.yaml
# → writes whetstone/rules/<lang>/<dep>.yaml with status: candidate

# 4. Approve candidates (single or batch)
wh rules approve --all --confidence high

# 5. Generate context, tests, and lint configs
wh actions all # chains wh actions context, test, and lint
# → whetstone/context/*, whetstone/evals/**, whetstone/lint/*

# 6. Verify source code against approved rules
wh scan src/

# 7. When dependencies drift, re-resolve only what changed
wh reinit              # writes whetstone/.state/refresh-diff.json
wh reinit --check      # same, but exits non-zero when drift is detected (CI-friendly)
```

> **Agent skill mode:** The skill is the front door. Say "wh init" or "extract rules" and the agent runs the full workflow — owning the judgment loop and calling the CLI for the deterministic work (detection, resolution, validation, generation, scan, drift gate).

### Worked Example: Extracting Rules for a Rust Project

Here's what a real run looks like on Whetstone's own codebase (Rust, 10 dependencies):

```bash
$ wh init
────────────────────────────────────────
  Whetstone Init — 2026-04-20
────────────────────────────────────────
  Dependencies: 16 runtime (+2 dev) across python, rust
  Sources:      10 resolved with content (README + changelog)
  Changelogs:   5 found (clap, chrono, rayon, reqwest, regex)
  Ready:        10 dependencies ready for extraction
────────────────────────────────────────
```

The skill reads the resolved content and proposes rules. Each enforceable rule
gets a deterministic backing; each one is presented as a card:

```
[MUST] anyhow.context-over-map-err — high confidence — convention
  Source kind: official_docs
  Prefer .context(...) over .map_err(|e| anyhow!(...)) for error context.
  Source: https://docs.rs/anyhow/latest/anyhow/trait.Context.html
  Signal: ast — map_err closure containing the anyhow! macro  [strategy: ast]
  > Approve / Edit / Skip?

[MUST] anyhow.expect-over-unwrap — high confidence — convention
  Source kind: official_docs
  Use .expect("...") over .unwrap() so panics carry context.
  Source: https://docs.rs/anyhow/latest/anyhow/
  Signal: lint_proxy — clippy unwrap_used  [emits clippy config]
  > Approve / Edit / Skip?
```

Not everything becomes a rule. Type-aware advice — e.g. "set an explicit
`reqwest` timeout" or "prefer the clap derive API" — needs to know a value's
type, which the tree-sitter scanner can't resolve. The skill **carries that as
guidance** instead of minting a brittle regex rule. (See
[`planning/skill-cli-boundary.md`](planning/skill-cli-boundary.md).)

You approve or skip each rule. Approved rules are written to
`whetstone/rules/rust/anyhow.yaml`.

Then generate outputs:

```bash
$ wh validate          # ✓ All schema checks passed
$ wh actions context   # → whetstone/context/AGENTS.md
$ wh actions lint      # → whetstone/lint/clippy.whetstone.toml (enables unwrap_used)
$ wh scan src/         # → tree-sitter enforces the ast rule; lint config the lint_proxy rule
$ wh status            # → Score: 95 | Label: Healthy
```

`wh scan` runs the `ast` rule via a tree-sitter query and verifies the
`lint_proxy` rule's native config — enforcement after code is written. Meanwhile
the generated context under `whetstone/context/AGENTS.md` tells your AI coding
agent the same rules (plus the skill's type-aware guidance) before code is
written — both halves from the same approved ruleset.

## Canonical Workflow

Whetstone follows a seven-step lifecycle. `wh init` handles steps 1 + 2 in one go.

| Step | Command | What happens |
|------|---------|-------------|
| **1. Detect** | `wh init` | Scan manifests for dependencies |
| **2. Resolve** | `wh init` | Resolve docs URLs from registries, probe for llms.txt |
| **3. Extract** | `wh extract` + agent | Agent reads docs, drafts a candidate bundle |
| **4. Submit** | `wh extract submit <bundle>` | Writes the bundle as `status: candidate` |
| **5. Approve** | `wh rules approve <id>` or `wh rules approve --all` | Flip candidates to approved |
| **6. Generate** | `wh actions all` | Run `wh actions context`, `wh actions test`, `wh actions lint` |
| **7. Monitor** | `wh status` / `wh ci` / `wh scan` / `wh debt` | Track freshness, drift, enforce rules, and triage deterministic debt hotspots |

When dependencies update, run `wh reinit` to re-resolve changed sources, then re-extract rules for what changed. `wh reinit --check` exits non-zero if drift was detected (useful in CI).

See [`references/workflow-matrix.md`](references/workflow-matrix.md) for the full command matrix, including every alias and which lifecycle step each command serves.

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│  Skill / Agent (front door, judgment)                       │
│    reads docs · proposes rules · carries taste guidance      │
│    orchestrates the workflow ─ and CALLS the CLI below ──┐   │
│                                                          ▼   │
│  Rust CLI (deterministic substrate)                          │
│    wh init    ─ detect deps, resolve docs, content-hash      │
│    wh validate ─ schema-check candidate rules                │
│    wh actions all ─ generate context + tests + lint config   │
│    wh scan    ─ enforce ast / lint_proxy rules               │
│    wh status / wh ci ─ health score + drift gate             │
│    wh debt    ─ deterministic debt hotspots                  │
└─────────────────────────────────────────────────────────────┘
```

**The skill is the front door and owns judgment:** reading documentation, proposing rules, carrying taste/type-aware guidance that can't be deterministically enforced, and orchestrating the loop. **The CLI is the deterministic substrate the skill calls:** dependency detection, URL resolution + content-hashing, validation, file generation, the AST/lint scanner, and the drift gate. The skill can be Claude, Cursor, Copilot, or any LLM — the CLI doesn't care. Nothing structural is removed; only the read-docs → draft-rules loop is skill-driven.

### What gets proposed

| Category | Example |
|----------|---------|
| Migration footgun | Pydantic deprecated `schema()` — use `model_json_schema()` |
| Non-obvious default | SQLAlchemy `create_engine()` pools connections by default |
| Convention divergence | FastAPI docs say `async def`, most tutorials use `def` |
| Breaking change | API will fail in next major version |
| Semantic practice | Error messages should include the invalid value |

### What gets rejected

- Generic advice ("write clean code")
- Subjective preferences without documentation backing

A check that a standard linter already covers (ruff/biome/clippy) is **not**
rejected — it becomes a `lint_proxy` rule that emits the native config to enable
it. Guidance that needs taste or type resolution (and so has no deterministic
signal) is **not** rejected either — it lives in the skill as agent guidance
rather than a signal-less rule.

## Commands

```bash
whetstone <command> [options]   # `wh` is the shorter alias
```

Shipped commands (canonical surface first):

| Command | Purpose | Key Flags |
|---------|---------|-----------|
| `init` | Bootstrap from zero: detect deps, resolve docs, write extraction handoff | `--changed-only`, `--refresh`, `--resume`, `--personal`, `--hooks`, `--ci`, `--ready-only` |
| `reinit` | Re-resolve changed dependencies/docs and emit refresh diff | `--check`, `--project-dir` |
| `status` | Project health summary, adherence, drift, and report generation | `--json`, `--score`, `--history`, `--no-drift-check`, `--report` |
| `scan` | Scan source files for rule violations and linter-config gaps | `<paths>`, `--lang`, `--rule`, `--no-fail` |
| `actions all` | Generate context, tests, and lint/formatter overlays in one chain | `--dry-run`, `--lang`, `--personal`, `--terse` |
| `actions context\|test\|lint` | Run one generator explicitly (`lint` also emits formatter-backed overlays when configured) | `--dry-run`, `--lang`, `--personal` |
| `rules ...` | Manage rules, approvals, and JIT rule lookup | `list`, `show`, `query`, `add`, `edit`, `remove`, `approve`, `worklist` |
| `sources ...` | Manage custom rule sources | `list`, `add`, `edit`, `remove`, `verify` |
| `config ...` | Inspect and validate the effective config stack and imported packs | `show`, `validate` |
| `extract` | Walk the extraction worklist or submit a candidate bundle | `submit <bundle.yaml>`, `--dep`, `--lang` |
| `approve` | Compatibility top-level approval entrypoint | `<rule-id>`, `--all`, `--dep`, `--confidence` |
| `debt` | Deterministic debt triage across dead code, dupes, deps, hotspots | `--json`, `--prompt`, `--beads`, `--top`, `--since` |
| `validate` | Validate the rule schema and rule fixtures | `--project-dir` |
| `update` | Update the `whetstone` binary to the latest release | `--check`, `--force` |

Project-scoped commands accept `--project-dir` (default: `.`), and all commands support `--json` (auto-enabled when piped). Human-readable progress goes to stderr. Many JSON responses include a `next_command` field suggesting what to run next.

For existing users migrating older scripts and habits to the canonical surface,
see [`references/cli-vnext-migration.md`](references/cli-vnext-migration.md).

### Canonical shareability story

The default shareable artifact is a committed **`whetstone/whetstone.yaml`**.
Teams should be able to copy that file into another repo, run Whetstone, and
get the same trusted-source setup and taste policy. Optional `extends:` pack refs
compose underneath that file; they do not replace it as the primary UX.

> **Python is not a runtime dependency.** Every user-facing command ships from the Rust binary. Archived Python reference implementations live under `scripts/legacy/` solely so `tests/test_script_contracts.py` can parity-test the Rust ports.

## Outputs

### Rule YAML files

Rules live in `whetstone/rules/{language}/{dependency}.yaml`:

```yaml
source:
  name: fastapi
  version: "0.115.0"
  content_hash: sha256:abc123

rules:
  - id: fastapi.async-routes
    severity: must            # must | should | may
    confidence: high          # high | medium
    category: convention      # migration | default | convention | breaking-change | semantic
    description: >
      Route handlers MUST use async def.
    source_url: https://fastapi.tiangolo.com/async/
    approved: true
    formatter:
      tool: ruff
      options:
        quote-style: single
    tests:
      - runner: pytest
        path: tests/style/test_fastapi_rules.py
        selector: test_async_routes
    signals:
      - id: is-sync-function
        strategy: ast         # ast | lint_proxy | pattern (deprecated)
        ast_query: ...        # required for `ast` signals (tree-sitter S-expression)
        weight: required
      - id: mutable-defaults
        strategy: lint_proxy
        weight: required
        lint:
          tool: ruff
          code: B006
```

See [`references/rule-schema.yaml`](references/rule-schema.yaml) for the full schema.

### Generated files

| Output | Location | Purpose |
|--------|----------|---------|
| Tests | `whetstone/evals/` | Native test files (pytest/vitest/cargo) |
| Lint / formatter configs | `whetstone/lint/` | Ruff/biome/clippy plus formatter-backed configuration fragments where rules opt into them |
| Agent context | `whetstone/context/` | Generated AGENTS.md / CLAUDE.md / .cursorrules / Copilot / Windsurf / Codex instructions |

### Status output

`wh status` returns a health score (0-100) with five dimensions:

| Dimension | What it measures |
|-----------|-----------------|
| `freshness_days` | Days since last rule extraction |
| `rules_count` | Total approved rules |
| `high_confidence_ratio` | % of rules with `confidence: high` |
| `deterministic_coverage` | % of signals with a deterministic backing (ast / lint_proxy / formatter / tests / validators) |
| `pending_updates` | Dependencies with version drift |

Labels: **Healthy**, **Needs Review**, **Stale**, **No Rules**.

### Impact metrics

`wh status` also includes a `metrics` object for tracking value over time:

| Metric | What it measures |
|--------|-----------------|
| `rules_approved` | Total approved rules |
| `rules_proposed` | Total rules proposed (including unapproved) |
| `approval_rate` | % of proposed rules that were approved |
| `must_rules` | Count of highest-severity (`must`) rules |
| `dependencies_covered` | Dependencies with at least one approved rule |
| `dependencies_total` | Total tracked dependencies |
| `dependency_coverage` | % of dependencies with rules |
| `deterministic_coverage` | % of signals with a deterministic backing (ast / lint_proxy / formatter / tests / validators) |
| `pending_drift` | Dependencies with version drift |

### Metric history

Every `wh status` run automatically appends a timestamped snapshot to `whetstone/.metrics.jsonl`. Use `--history` to see trends:

```bash
wh status --history
```

This shows a table of score, label, rules count, and drift over time. Use `--no-snapshot` to skip recording (e.g., in scripts that poll status without wanting to inflate history).

**Anti-gaming guidance:** Metrics reflect the state of your rules, not your code quality. A high score with 5 well-chosen rules is better than a high score with 50 trivial rules. Focus on the `must_rules` and `deterministic_coverage` metrics — these indicate rules that catch real mistakes with real checks. The `approval_rate` metric helps calibrate extraction quality: if it's consistently low, your extraction prompt may need tuning.

### Debt triage

`wh debt` complements `wh status` by surfacing deterministic maintainability hotspots caused or amplified by AI-assisted code generation:

- dead code
- duplicate blocks
- dependency hygiene issues
- churn × violations hotspots

Useful modes:

```bash
wh debt --json                 # stable machine-readable envelope
wh debt --prompt               # compact remediation handoff for another agent
wh debt --beads --json         # file a bd epic + child tasks and return created ids
wh debt --since=90d --top=10   # tune hotspot window + list length
```

## Use with coding agents (MCP)

`whetstone mcp` runs a local [Model Context Protocol](https://modelcontextprotocol.io)
stdio server that exposes two deterministic tools to any MCP-capable agent
(Claude Code, Cursor, …) — no bespoke wiring:

- **`rules_query`** — the rules that apply to a file/dep/language (call it before editing a file).
- **`scan`** — scan paths for violations of the approved rules (self-check before finishing).

Register it with Claude Code:

```bash
claude mcp add whetstone -- whetstone mcp --project-dir /path/to/your/repo
```

or in an MCP client config:

```json
{
  "mcpServers": {
    "whetstone": { "command": "whetstone", "args": ["mcp", "--project-dir", "."] }
  }
}
```

The agent then gets JIT, deterministic, doc-cited rules mid-edit and can self-check
its own output — the same JSON the CLI produces, with no tokens spent re-deriving
conventions each session.

## CI Integration

`wh init --ci` generates `.github/workflows/whetstone-check.yml` with two
agent-free gates:

- **enforce** (push / pull_request) — `wh scan` fails the build on violations of
  rules the scanner enforces directly (AST rules). No agent, no tokens.
- **freshness** (schedule) — `wh ci` fails on content-hash / version drift (the
  docs a rule was derived from changed since it was authored).

For rules delegated to a linter (`lint_proxy`), run `wh actions lint` and merge
the generated native config so your normal lint CI enforces them too:

| Ecosystem | Generated overlay | How to wire it |
|-----------|-------------------|----------------|
| Rust | `whetstone/lint/clippy.whetstone.toml` (`[lints.clippy]`) | merge into your `Cargo.toml` `[lints]` — Cargo applies it on every `cargo clippy` |
| Python | `whetstone/lint/ruff.whetstone.toml` | `extend = "whetstone/lint/ruff.whetstone.toml"` in your `ruff.toml`/`pyproject.toml` |
| TypeScript | `whetstone/lint/biome.whetstone.json` | add to your `biome.json` `extends` |

Whetstone decides *which* rules are worth enforcing; your existing linters enforce
the ones they can express, and `wh scan` enforces the rest.

### GitHub Action

```yaml
# .github/workflows/whetstone.yml
name: Whetstone Check
on:
  pull_request:
    branches: [main]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: whetstone/whetstone@main
        id: whetstone
        with:
          changed-only: "true"
          fail-on: stale
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

#### Action migration note

Older Whetstone revisions used Python scripts inside the GitHub Action.
The current action builds and runs the Rust binary directly. If you previously
depended on Python internals, migrate to the documented action inputs/outputs in
`action.yml` instead of shelling out to a script.

Action inputs:

| Input | Default | Description |
|-------|---------|-------------|
| `directory` | `.` | Project directory to check |
| `changed-only` | `true` | Only check dependencies with version drift |
| `fail-on` | `none` | Exit with error on: `stale`, `needs_review`, or `none` |
| `github-token` | — | GitHub token for posting PR comments |
| `python-version` | `3.11` | Python version to use |

Action outputs: `freshness_status`, `changed_sources_count`, `recommended_rules_count`, `requires_review`, `score`.

## Languages

| Language | Manifest | Registry | Tests | Linter | Support Tier |
|----------|----------|----------|-------|--------|--------------|
| Python | `pyproject.toml`, `requirements.txt` | PyPI | pytest | ruff | **Full** — reference implementation |
| TypeScript | `package.json` | npm | vitest | biome | **Baseline** — common signals work, complex patterns scaffold |
| Rust | `Cargo.toml` | crates.io | cargo test | clippy | **Baseline** — common signals work, complex patterns scaffold |

### What each tier means

**Full (Python):** tree-sitter `ast_query` checks for function signatures, decorators, imports, class inheritance, keyword arguments. `ast_scope`-bounded text checks for string-literal content and naming. Ruff overlay generation for `lint_proxy` signals. Generated tests are complete and runnable.

**Baseline (TypeScript, Rust):** tree-sitter `ast_query` checks for import structure and deprecated-call shape, plus `lint_proxy` overlay generation (biome/clippy). Generated tests work for these signal types. Type-dependent checks (type inference, trait bounds, "is this receiver a `reqwest::Client`?") are **not** scaffolded — tree-sitter has no type resolution, so they're carried as skill guidance instead of a CLI rule (see [`planning/skill-cli-boundary.md`](planning/skill-cli-boundary.md)).

## Privacy

Pattern mining from agent transcripts was available in earlier planning and may
return later alongside a signal-promotion pipeline, but it is **not part of the
current shipped local flow**.

Today, Whetstone's shipped workflow is driven by:

- dependency manifests
- trusted subscribed sources
- approved local rules

No transcript-mining pass is part of the default runtime.

### Private mode — adopt solo on a shared repo

Want Whetstone on a team repo where nobody else has opted in yet? Onboard in
private mode and **nothing shows in `git status`** for your teammates:

```bash
wh init --claude --private
```

Artifacts live at their normal paths, but a managed block in `.git/info/exclude`
(per-clone, never committed) hides them. Whetstone then asks git whether that
actually worked and **fails loudly** if anything is still visible, so "private"
is verified rather than assumed. Note that worktrees of one clone share the
exclude file: `wh publish` in any of them makes the artifacts visible in all. Enforcement is unchanged — the hook,
MCP server, and `wh scan` all work exactly as in public mode. Private mode never
modifies files the repo already tracks: a committed `.mcp.json` is left alone
(you get the `claude mcp add -s local` alternative), and hooks go to
`.claude/settings.local.json` when `.claude/settings.json` is tracked.
`--ci` is refused while private — a workflow file is inherently shared.

When the team is ready:

```bash
wh publish        # optionally: --ci
```

This removes the exclude block, adds the real `.gitignore` entries for
machine-local state, completes the shared wiring, and prints the `git add` list.
It never stages or commits anything — sharing stays your explicit move.

Two honest caveats. `wh init --hooks` may set `core.hooksPath` in your local
`.git/config` so the post-merge hook fires (per-clone, never committed, and
skipped when your repo already has live hooks). And publishing is one-way: the
`.gitignore` entries it writes are a legitimately shared file, so re-entering
private mode afterwards leaves that one visible change.

## How Whetstone Fits with Existing Tools

Whetstone is designed to complement — not replace — your existing toolchain.

| Tool | What it does | How Whetstone complements it |
|------|-------------|------------------------------|
| **ruff / biome / clippy** | Enforces syntax, formatting, and general code quality rules | Whetstone catches dependency-specific practices these linters don't know about. Where a linter rule exists but isn't enabled, Whetstone generates a lint overlay to enable it. |
| **PR review bots** (reviewdog, danger, etc.) | Automated checks on pull requests | Whetstone generates the rules these bots enforce. Run `wh ci` in CI for freshness gating alongside your existing checks. |
| **AI code review** (CodeRabbit, Copilot review, etc.) | LLM-powered code review | Whetstone provides deterministic, source-backed rules that don't vary between runs. Use it for the checks you want to enforce consistently, AI review for everything else. |
| **`whetstone/context/*`** | Static agent instructions | Whetstone auto-generates and keeps these files current. When dependencies update, your agent instructions update too. |
| **Semgrep / CodeQL** | Custom static analysis rules | For TypeScript and Rust, Whetstone can generate signal patterns that map to Semgrep rules. For Python, Whetstone's pytest-based checks are simpler to maintain. |

### What Whetstone adds that nothing else does

1. **Source-backed provenance** — every rule cites a specific documentation URL
2. **Drift detection** — knows when your dependencies updated and your rules didn't
3. **Multi-output from single source** — same approved rule becomes a test, a lint config, and an agent instruction
4. **Recency awareness** — prioritizes rules about recent changes that LLMs weren't trained on

## FAQ

**How is this different from a linter?**
Linters enforce syntax and formatting rules. Whetstone catches dependency-specific practices that linters don't know about — migration footguns, non-obvious defaults, convention divergence. It generates linter config fragments where possible, and native tests for everything else.

**Do I need an LLM API key?**
No. Whetstone is an Agent Skill — the agent running it (Claude, Cursor, etc.) acts as the LLM. No separate API key or binary required.

**What if Whetstone doesn't find any rules for my dependency?**
That's correct behavior. If the documentation doesn't clearly state practices worth enforcing, Whetstone stays silent. You can always add rules manually.

**Can I add custom sources beyond dependency docs?**
Yes. Add any URL to `whetstone/whetstone.yaml` and Whetstone fetches it alongside registry sources:

```yaml
sources:
  custom:
    - url: https://team-guide.internal/rust-conventions
      name: "Team Rust Guide"
      source_kind: team_guide
    - url: https://blog.example.com/fastapi-pitfalls
      name: "FastAPI Pitfalls"
      source_kind: blog
```

Custom sources appear in `wh init`, `wh sources list`, and the extraction worklist. Each rule you extract from them keeps source provenance for filtering and review.

You can also point Whetstone at a local markdown second brain / wiki vault:

```yaml
sources:
  vaults:
    - id: team-brain
      path: docs/brain
      include: ["**/*.md"]
      source_kind: second_brain
      authority: reviewed
```

Vault pages can carry frontmatter with authority, language, dependency, and
upstream-link metadata. Whetstone indexes the vault into
`whetstone/.state/knowledge-graph.json`, preserves page provenance on derived
rules, and surfaces related pages in extraction context.

**What happens if I don't install Whetstone?**
Nothing breaks. The generated tests, lint configs, and agent context files are standard files in your repo. They run with your existing CI, and the generated agent context lives under `whetstone/context/` (or `whetstone/.personal/context/` for personal-only output).

**How do I update rules when dependencies change?**
Run `wh status` or `wh ci` to see which dependencies have drifted. Then run `wh reinit` (or `wh init --changed-only`) to re-resolve only what changed, and re-extract rules against the new content. Use `wh reinit --check` in CI to fail a build when drift is detected.

**What's the `next_command` field in some outputs?**
Many workflow-driving commands suggest what to do next. Agent clients can use this to chain canonical commands automatically without rereading documentation.

## Self-Hosting (Dogfooding)

Whetstone can be used on itself. The `tests/fixtures/` directory contains sample manifests that demonstrate the full workflow. To run the self-hosting workflow:

```bash
# Bootstrap against the test fixtures
wh init --project-dir tests/fixtures --json

# Check status of existing rules
wh status --project-dir tests/fixtures

# Generate sample outputs from the rules
wh actions test --project-dir tests/fixtures --dry-run
wh actions context --project-dir tests/fixtures --dry-run
```

The test fixtures include rule files for fastapi and react that demonstrate the full rule schema with lifecycle fields, provenance metadata, and golden examples. This serves as a reference for the quality bar Whetstone expects.

## Current Capabilities vs Roadmap

**Shipped today (0.3.0):**
- Dependency detection across Python, TypeScript, and Rust (including monorepos)
- 4-tier content resolution: llms.txt → registry README → HTML docs → GitHub changelog
- Changelog fetching with 18-month recency filtering
- Custom source URLs in `whetstone.yaml` (blogs, team guides, any public URL)
- Local second-brain / markdown vault ingestion with authority metadata and graph indexing
- Agent-mediated rule extraction via `wh extract` + bundle submission (`wh extract submit`)
- Bulk approval via `wh rules approve --all [--dep] [--confidence]`
- Tree-sitter-backed `wh scan` across Python, TypeScript, and Rust, including AST-query and AST-scoped regex enforcement
- Regex-backed `wh scan` for HTML/CSS/JavaScript profiles and command-validator execution for custom checks
- Rule listing and per-rule context via `wh review` / `wh rules show`
- Test generation with real regex checks (via `match` field on signals) for Python, TypeScript, and Rust
- Lint overlay generation (ruff, biome, clippy) via `wh lint`
- One-shot generation chain via `wh actions all` (context + tests + lint)
- Agent context generation under `whetstone/context/` (AGENTS.md, CLAUDE.md, .cursorrules, copilot, windsurf, codex)
- Health monitoring with drift detection, freshness scoring, and metric history
- CI integration via GitHub Action with PR comments
- Drift-based refresh command (`wh reinit` / `wh reinit --check`) with reviewable diff artifact
- Personal + project layer rule merge with auto-gitignored personal overrides
- **Advisory automation hooks** — `wh init --hooks` installs a post-merge git hook + Claude Code `SessionStart` advisory; `wh init --ci --schedule=<cadence>` generates a scheduled GitHub Actions freshness check
- Binary self-update via `wh update`

**Deferred (0.3.0 lean refactor):**
- `wh promote` / `wh layers` — team and built-in layers were removed.
- `wh propose` / `wh apply` / `wh review queue|diff` — replaced by extract + approve.
- `wh bench` / `wh patterns` — benchmark corpus and pattern mining are parked. (`wh eval` is NOT parked — it is the current golden↔scanner rule-quality bar.)
- Built-in rules and team `extends:`.

**Planned:**
- ast-grep pattern generation (structural enforcement via CodeRabbit-compatible rules)
- MCP server for agent-native rule queries
- Shared rule registry with community-ranked rules

See [`planning/whetstone-overview.md`](planning/whetstone-overview.md) for the current overview and [`references/workflow-matrix.md`](references/workflow-matrix.md) for the command-to-lifecycle mapping.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `No manifests found` | Ensure `pyproject.toml`, `package.json`, or `Cargo.toml` exists in your project directory |
| `status: not_initialized` | Run `wh init` first to detect deps and create the `whetstone/` directory |
| Drift check is slow | Use `--no-drift-check` for faster status, or `--changed-only` to limit scope |
| Rules from stale docs | Check `source_url` in your rule YAML — Whetstone flags when source content changes via `content_hash` |

---

*Whetstone sharpens the tools that write your code.*
