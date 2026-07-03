# Whetstone Skill ↔ CLI Boundary

> The authoritative contract for what is **judgment** (skill / agent) vs **deterministic** (CLI),
> and the discipline that keeps the rule/signal audit aligned going forward.
>
> Status: design doc for epic `whetstone-4xw`. Date: 2026-06-28.

## 1. Decision

Whetstone is **skill-first with a thin deterministic CLI**, in the **role-thin** sense:

- The **skill** is the front door. It owns *judgment*: reading dependency docs, proposing
  high-confidence rules (taste), orchestrating the workflow, and carrying soft/taste guidance
  that is not deterministically enforceable.
- The **CLI** keeps all its *deterministic* commands — detection, resolution + content-hashing,
  schema validation, generation of native lint/test/context artifacts, the AST/lint scanner, and
  the drift gate. The skill **calls** these; they are not removed.
- "Thin" therefore means **role + discipline, not line count.** The only structural change is that
  the *read-docs → draft-rules* loop becomes skill-driven (the deterministic `wh extract` worklist
  and `wh extract submit` validation remain as primitives the skill calls).

Rationale: enforcement-without-an-agent does not require a binary — a skill can emit native linter
configs that run agent-free in CI. The binary's only durable moat is (a) custom AST/semantic
enforcement no linter can express and (b) stateful drift detection. Keeping deterministic
*generation* in the CLI also preserves determinism that a skill hand-writing configs would lose.

> **Design-review caveat (2026-06-28, see §9).** Two load-bearing claims here are narrower or
> currently broken than first written: the custom-AST moat (a) is limited by tree-sitter's lack of
> type resolution, and "native configs run agent-free in CI" does **not** yet hold for Rust/clippy.
> Both are tracked as fixes in §9 and the epic. The *direction* stands; the moat is smaller.

## 2. Division of labor

| Concern | Owner | Why |
|---|---|---|
| Read docs, judge which practices matter, draft candidate rules | **Skill** | Requires judgment |
| Soft/taste guidance with no deterministic signal | **Skill** | Not gateable |
| Orchestrate the extract → approve → generate workflow | **Skill** | Front door |
| Dependency detection, source resolution + content-hash, caching | **CLI** | Deterministic |
| Rule-schema validation (`wh validate`) | **CLI** | Deterministic |
| Generate native lint/formatter configs, tests, agent context | **CLI** | Deterministic; skill calls it |
| Custom AST/semantic enforcement (`wh scan`) | **CLI** | No linter expresses it |
| Drift / freshness gate (`wh status`, `wh ci`) | **CLI** | Stateful, scheduled |
| Approve / reject a candidate | **User** (via CLI) | Policy decision |

## 3. The signal audit discipline (the contract that must hold forever)

Every rule signal falls into **exactly one** of three buckets. The fourth (regex) is retired.

1. **Drop — duplicates an existing tool → emit native config.**
   - If expressible by ruff/biome/clippy → `strategy: lint_proxy` with `lint: {tool, code}`.
     `wh actions lint` then emits the native config that enables it. Do **not** delete and do
     **not** reimplement as regex.
   - If the dup is in a tool `lint_proxy` does not support (cargo-audit/RUSTSEC, pip-audit,
     npm audit, type-checkers) → **either** delete the rule and document "use cargo-audit"
     **or** bind via `validators: command` (e.g. `cargo audit`). Default: delete + document.

2. **Move to skill — needs taste or type resolution.**
   - Lives as agent guidance in the skill, with **no** deterministic signal and **no** CLI rule.
   - Example: "derive vs builder API" needs to know whether `Command` is `clap::Command` or
     `std::process::Command` — type resolution the scanner does not do. It is taste, not a gate.

3. **Keep as AST — no linter expresses it, AND it is expressible WITHOUT type resolution.**
   - `strategy: ast` with a real `ast_query` (tree-sitter S-expression). This is the CLI's moat —
     but a **narrow** one. tree-sitter matches *syntax structure*, not types: it cannot answer
     "is this receiver a `reqwest::Client`?" and has no native "subtree lacks node X" operator.
   - Bucket 3 is therefore limited to **type-independent structural** checks — decorator shape,
     async-vs-sync function form, import structure, presence of a node in a clearly-identified
     construct.
   - Rules that need a value's **type**, or that a specific library's chain is **missing** a call,
     are NOT bucket 3. They move to the skill (bucket 2), where the agent can reason about types.
     (This re-buckets the reqwest rules — see §4 and §9.)

**Retired bucket — top-level `strategy: pattern` (raw regex).** Brittle and the source of the
current credibility gap. Deprecated in the schema; `wh validate` rejects it, with a narrow
allowlist for genuinely text-level checks (string-literal content, naming) where AST adds nothing.
**This does not make AST "regex-free":** tree-sitter `#match?` / `#not-match?` predicates and
`ast_scope`-bounded regex remain available *inside* `ast` signals, and absence/negation checks
still rely on them. The win is that regex becomes **scoped to a parsed node** rather than a blind
text sweep. `wh validate` cannot police regex hidden inside `ast_query` predicates — human review
must (see §9 M1).

## 4. Per-rule disposition (current shipped rules)

| Rule | Today | Disposition | Mechanism |
|---|---|---|---|
| `anyhow.context-over-map-err` | `pattern` `.map_err…anyhow!` | **Keep as AST** | `ast_query` (no linter does this; regex too fragile across newlines) |
| `anyhow.expect-over-unwrap` | `pattern` `\.unwrap\(\)` | **Drop** | `lint_proxy` clippy `unwrap_used` → emit clippy config |
| `clap.derive-over-builder` | `pattern` `(Command\|App)::new\(` | **Move to skill** | taste guidance (false-positives on `std::process::Command`; `App` dead in clap 4; needs type resolution) |
| `reqwest.set-timeout` | `pattern` `Client::new\(\)` | **Move to skill** | needs type resolution (is the receiver a `reqwest::Client`?) + absence-in-chain → only expressible via `#not-match?` regex, which false-positives on unrelated builders. Same gap as clap. (Was "Keep as AST"; re-bucketed per §9 B1.) |
| `reqwest.check-status` | `pattern` `\.send…\.text\(` | **Move to skill** | same type-resolution + absence gap as `set-timeout`. (Was "Keep as AST"; re-bucketed per §9 B1.) |
| `serde_yaml.crate-deprecated` | `pattern` `serde_yaml` | **Drop** | not `lint_proxy`-expressible → delete + document "use cargo-audit/RUSTSEC" (or `validators: command`) |

Fixtures using `strategy: pattern` (e.g. `fastapi.async-routes` `\bdef `, `react`) migrate to
`ast_query` as part of the schema change.

## 5. CLI command classification (role-thin)

Default is **keep** (deterministic). Only one command reshapes; none is removed.

| Command | Disposition |
|---|---|
| `init`, `reinit`, `set-sources`, `sources`, `config` | Keep — deterministic setup/config |
| `validate` | Keep + **enforce no-regex** (rejects `strategy: pattern`) |
| `context`, `tests`, `lint`, `actions` | Keep — deterministic generation; skill calls them; `lint` becomes the primary enforcement path |
| `scan` | Keep — deterministic enforcement; **AST + lint_proxy only** |
| `eval` | Keep — deterministic rule-quality bar (goldens through the scanner). Added by whetstone-5co. |
| `mcp` | Keep — deterministic oracle transport (MCP stdio: `rules_query` + `scan`). Added by whetstone-b5b. |
| `hook posttooluse` | Keep — deterministic in-session enforcement adapter: scan the edited file, feed violations back to the agent. Added by whetstone-cpt. The judgment stays in the agent; the CLI just reports. |
| `status`, `ci` | Keep — drift/freshness gate (a CLI moat) |
| `rules`, `approve`, `review` | Keep — deterministic state/query ops |
| `report`, `debt`, `update` | Keep — deterministic, lower priority |
| `extract` (worklist) + `extract submit` | Keep as primitives; the **doc-reading/drafting loop around them moves to the skill** |
| TUI (intro splash, screens) | Keep but **deprioritize** — non-core under skill-first; stop investing |

## 6. Schema + validator changes

- `references/rule-schema.yaml`: mark `strategy: pattern` **deprecated**; document the narrow
  allowlist (string-literal / naming) and `ast_scope` as the bounded form. Remove the dead `ai`
  strategy reference everywhere it survives.
- `wh validate`: reject `strategy: pattern` outside the allowlist; require `ast` signals to carry
  `ast_query` (no silent regex fallback).
- Reconcile lifecycle to **`candidate | approved` only** across schema + docs (remove
  `denied`/`deprecated`).

## 7. Docs reconciliation checklist

The docs are currently **wrong and self-contradictory**; fixing them is part of "the correct
vision," independent of the reorientation.

- [ ] `SKILL.md` — drop "the binary is the sole runtime"; reposition skill as front door;
      reconcile "every rule needs a signal" vs schema "signals optional" (taste lives in the skill).
- [ ] `AGENTS.md` — remove dead `ai` strategy, dead `denied`/`deprecated` states, dead
      `wh propose` / `proposal-schema` references.
- [ ] `CLAUDE.md` — remove "sole runtime"; remove dead `wh patterns`.
- [ ] `README.md` — remove dead `wh patterns` / `wh propose`; lead skill-first.
- [ ] `references/signal-strategies.md` — retire regex-as-first-class; teach `ast_query` + `lint_proxy`.
- [ ] `references/rule-schema.yaml` — deprecate `strategy: pattern`; drop `ai`.
- [ ] `references/extraction-prompt.md`, `references/workflow-matrix.md` — align to the above.
- [ ] Live `planning/` docs — `whetstone-overview.md`, `whetstone-logic-flow.mmd`,
      `command-taxonomy.md` (dead commands, CLI-first framing). Archived `planning/archive/*` may
      be left or marked superseded.

## 8. Alignment criteria (definition of done for the epic)

We are "completely aligned" when **all** hold:

1. Every shipped signal is `lint_proxy`, `ast` (+`ast_query`), `formatter`, `tests`, or
   `validators` — **zero** raw `strategy: pattern` outside the allowlist.
2. `wh validate` enforces #1, so new rules cannot reintroduce regex.
3. Every "duplicates a linter" case emits native config (`lint_proxy`) or is documented as
   owned by another tool; none reimplemented in-house.
4. Taste/type-resolution rules live in the skill with no CLI rule.
5. The skill is the front door and owns the extraction loop; no deterministic CLI command removed.
6. Drift detection is proven as a real CI gate.
7. All docs in §7 describe this vision with no internal contradictions and no dead commands.

## 9. Open risks (design review, 2026-06-28)

An independent review built the binary and ran the queries. Findings, with the bead that owns each:

- **B1 (blocker) — the AST moat is narrower than first claimed.** tree-sitter has no type
  resolution and no "subtree lacks node X" operator, so `reqwest.set-timeout` / `check-status`
  cannot be clean bucket-3 AST rules (they need to know the receiver is a `reqwest::Client` and
  detect an absent `.timeout()`). **Re-bucketed to the skill** (§3, §4). Bucket 3 survives for
  *type-independent structural* rules — e.g. `anyhow.context-over-map-err` (a `map_err` whose
  closure contains the `anyhow!` macro) and `fastapi.async-routes` (decorator + sync-`def` form)
  are still feasible. Owner: **whetstone-bry** (define feasible bucket-3 scope) + **whetstone-oos**.

- **B2 (blocker) — Rust native config is inert.** `clippy.whetstone.toml` `warn = […]` does
  nothing; clippy lint *levels* require Cargo.toml `[lints.clippy]` / attributes / `--warn`, not
  `clippy.toml`. Overlays aren't merged into any lint invocation/CI, and clippy verification is
  skipped in `src/check/lint_proxy.rs`. So "drop → lint_proxy" yields an unverified, non-working
  config for Rust — breaking the agent-free-CI claim. Owner: **whetstone-480** (expanded).

- **M1 (major) — "regex retired" is partial.** Negation/absence and `ast_scope` still use regex
  predicates inside `ast` signals; `wh validate` can't police them. Accept regex *scoped to a
  parsed node* as legitimate; rely on review, not validation, for predicate quality. Owner:
  **whetstone-oos** (document the real rule).

- **M2 (major) — retiring `pattern` breaks authoring + existing rules.** `wh rules add --match`
  mints `strategy: pattern` (`src/rule_authoring.rs`), and personal rules already use it. The
  allowlist is undefined and there's no migration. Owner: **whetstone-oos** (define allowlist
  concretely; migrate `wh rules add`; migrate fixtures + personal rules).

- **M3 (major) — drift gate mis-scoped.** `wh status` / `wh ci` compute *version + 30-day time*
  drift, not content-hash; content-hash drift lives only in `handoff.rs` and needs a network
  re-fetch; `wh ci` defaults `fail_on=none`. The "already wired" premise was wrong. Owner:
  **whetstone-25r** (rewritten: wire handoff content-hash → ci, set a sane default, handle CI
  network).

- **Minor** — ruff/biome overlays also aren't auto-wired into project config/CI (the verifier at
  least surfaces unmerged rules as `config_issues`); bucket-1 lint_proxy rules consume the
  "max 5 rules/dep" budget to flip linter flags (partly justified by carrying provenance/source).
  Folded into **whetstone-480** / noted.

## 10. Scope-discipline decision — "on task" vs "on standard" (whetstone-bws)

"Keep agents on task" has two readings, and conflating them dilutes the product:

1. **on-STANDARD** — the code the agent writes follows the project's rules
   (deprecated APIs, conventions, taste). This is Whetstone's entire domain: a
   cited ruleset → deterministic scan → native config / CI / the in-session hook.
2. **on-TASK / scope discipline** — the agent doesn't wander: no refactoring
   unrelated files, no gold-plating, no drifting from the requested change.

**Decision (2026-06-29): scope discipline is OUT of scope for Whetstone.** Rationale:

- It's a fundamentally different mechanism — **diff-vs-declared-intent**, not
  source-vs-rules. It needs a task/intent declaration and a diff comparison, not a
  ruleset and a tree-sitter scanner. Nothing in Whetstone's substrate answers "did
  this edit belong to the task."
- Whetstone's value proposition doesn't transfer: "touched a file outside the
  task" is not a coding standard you can cite a doc for or express as an
  `ast_query`. High-confidence-or-silence, doc-backed rules simply don't model it.
- Whetstone is **the rule-intelligence layer** — governance of code *standards*.
  Scope discipline is plan/diff governance: adjacent, but a distinct product.

**Not architecturally impossible, just separate.** The PostToolUse hook substrate
(whetstone-cpt) *could* host a future scope check (compare the edited path against
a declared task scope), so if this is ever pursued it should be a **separate epic**
with its own design — never smuggled in under "keep agents on task."

**Consequence for messaging:** README/SKILL wording frames Whetstone as keeping
agents **on standard** / writing best-practice code, not "on scope." Do not
over-promise scope enforcement.

## 11. TUI role decision (whetstone-9ef)

Decision (2026-06-29): under skill-first, the **TUI is a secondary, optional human
entry point — not the front door** (the skill is). Investment is **frozen**: no new
"operator workbench" build. The bar is a **minimal usability floor**, not feature growth:

- It must remain coherent: consistent navigation/footers, no dead or placeholder
  screens, and it must not crash or show stale state.
- It may surface read-only status/browse (rules, sources, drift) and the existing
  guided rule-authoring forms, but new human-facing capability should land in the
  CLI oracles + the skill, which any agent or script can drive — not in bespoke TUI.
- Rationale: the deterministic value (scan/eval/drift) and the judgment value
  (skill) are both reachable headless; a heavy TUI would re-create that surface for
  one client. The recent intro-splash churn is exactly the kind of investment to avoid.

If a concrete, repeated human workflow emerges that the CLI/skill can't serve, revisit
with a specific, scoped proposal — not a general "make the TUI nicer" pass.

## 12. Resolution log — design-review items

### Resolution (post-validation, 2026-06-28)

A second independent validation found and we then fixed:

- **M2 (authoring consistency)** — `wh rules add` now takes `--ast-scope` and emits a bounded
  pattern; a bare `--match` into the project layer is refused up front with guidance (it would
  otherwise mint a file `wh validate` rejects). Personal/advisory bare `--match` still allowed.
  Tests cover all three paths.
- **480 / §8.6 (CI wiring)** — the generated `wh init --ci` workflow now gates on
  `--fail-on=needs_review` (so it fails on content **and** version drift, not just stale), and its
  header states the enforcement model: Whetstone generates native `[lints.clippy]`/ruff/biome
  config; the project's existing lint CI enforces it agent-free. (Running the linters inside the
  generated workflow — setting up each toolchain — is deliberately left to the project's own lint
  CI, which is where "the tools you already use" run.)
- **TS bucket-3 demonstrator** — added a committed TypeScript `ast_query` test (window/document
  member access) and gave the react fixture's `ast` signal a real query. Rust (`anyhow`), Python
  (`snake`), and TypeScript now each have a proven AST query. Remaining bare-`pattern` *fixture*
  signals stay advisory by design (only shipped rules under `whetstone/rules/` must be clean).
