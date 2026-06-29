# Signal Strategies Reference

> How Whetstone decomposes rules into deterministic checks — and what belongs in
> the skill instead.

## Overview

A Whetstone rule earns a place in the ruleset only when it has a **deterministic
backing**: an `ast` signal (with a real `ast_query`), a `lint_proxy` signal, or a
`formatter` / `tests` / `validators` binding. Guidance that can't get one — because
it needs taste or type resolution — is not a signal-less rule. It lives in the
**skill** as agent guidance. The authoritative split is in
[`../planning/skill-cli-boundary.md`](../planning/skill-cli-boundary.md).

## The signal audit: three buckets

Before writing a signal, put the candidate in **exactly one** bucket.

### Bucket 1 — Duplicates an existing tool → emit native config

If a linter already expresses the check, don't reimplement it and don't drop it.
Bind it to the tool so Whetstone emits the native config that enables it.

- Expressible by ruff/biome/clippy → `strategy: lint_proxy` with `lint: {tool, code}`.
  `wh actions lint` then writes the native config fragment.
- The check belongs to a tool `lint_proxy` doesn't support (cargo-audit/RUSTSEC,
  pip-audit, npm audit, type-checkers) → either delete the rule and document "use
  cargo-audit" / "use pip-audit", **or** bind it via `validators: command`
  (e.g. `cargo audit`). Default: delete + document.

### Bucket 2 — Needs taste or type resolution → move to the skill

If verifying the rule requires judgment, or requires knowing a value's **type**,
it cannot be a deterministic CLI rule. tree-sitter matches *syntax structure*, not
types: it cannot tell a `reqwest::Client` from a `std::process::Command`, and it
has no native "this subtree is missing node X" operator. Such guidance lives in
the skill (no signal, no CLI rule). Examples: "set a reqwest timeout" (needs to
know the receiver's type and that `.timeout()` is absent), "prefer clap derive
over builder" (needs to know `Command` is `clap::Command`).

### Bucket 3 — No linter expresses it, and it's type-independent → keep as AST

`strategy: ast` with a real `ast_query` (a tree-sitter S-expression). This is the
CLI's moat, but a **narrow** one: only *type-independent structural* checks —
decorator shape, async-vs-sync function form, import structure, the presence of a
node inside a clearly-identified construct.

## Strategy types

### `ast` — tree-sitter structural query (the default deterministic strategy)

**What it does**: Parses source into a syntax tree and matches structure with a
tree-sitter S-expression `ast_query`.

**When to use**:
- Function signatures (async vs sync, parameters, return shape)
- Decorator presence or absence
- Class inheritance patterns
- Import statements and their structure
- A method call's syntactic shape inside an identified construct

**Deterministic**: Yes — same result for the same code.

**Implementation**: tree-sitter across Python, TypeScript, and Rust. Every `ast`
signal must carry an `ast_query`; `wh validate` rejects an `ast` signal with no
query (there is no silent regex fallback).

```yaml
signals:
  - id: sync-route-handler
    strategy: ast
    description: Route handler declared with sync def instead of async def
    weight: required
    ast_query: |
      (function_definition name: (identifier)) @fn
```

**`ast_query` is not "regex-free."** Absence/negation and text-level refinements
still use tree-sitter `#match?` / `#not-match?` predicates and `ast_scope`-bounded
regex *inside* the `ast` signal. The win over raw regex is that the match is
**scoped to a parsed node** rather than a blind text sweep. `wh validate` cannot
police regex hidden inside an `ast_query` predicate — that is a human-review
responsibility.

| Check | Structural target |
|-------|-------------------|
| Function is async | `async` keyword on the function node |
| Has decorator X | decorator child of the function node |
| Imports from module | import node with the module name |
| Class inherits from X | base-class child of the class node |
| `map_err` closure contains `anyhow!` | call node whose closure subtree holds the macro |

### `lint_proxy` — delegate to an existing linter

**What it does**: Maps the rule to a ruff/biome/clippy lint that isn't on by
default, so Whetstone emits the native config that turns it on.

**When to use**: any time a linter already expresses the check (bucket 1). This is
the **primary enforcement path** — prefer it over hand-rolling an AST query.

**Deterministic**: Yes (delegated to the linter).

**Implementation**: Always use structured `lint` metadata, never free-text:

```yaml
signals:
  - id: mutable-defaults
    strategy: lint_proxy
    description: Covered by Ruff
    weight: required
    lint:
      tool: ruff
      code: B006
```

`wh actions lint` then generates the native overlay:
- Python: `ruff.whetstone.toml` (`extend-select`)
- TypeScript: `biome.whetstone.json` (rule config)
- Rust: `clippy.whetstone.toml` (clippy lint settings)

| Check | Linter rule |
|-------|-------------|
| Unused function arguments | ruff `ARG001` |
| Mutable default arguments | ruff `B006` |
| Use of `any` type | biome `noExplicitAny` |
| `.unwrap()` without `.expect()` | clippy `unwrap_used` |

### `formatter` / `tests` / `validators` bindings

When a rule maps to a mechanical rewrite or an existing check rather than a
scanner signal, bind it directly instead of inventing a signal:

- `formatter` — a safe mechanical rewrite owned by a formatter (e.g. ruff format
  / rustfmt / biome) via `formatter: {tool, options}`.
- `tests` — an existing test that proves the rule via `tests: [{runner, path, selector}]`.
- `validators` — an external command that owns the check via `validators: command`
  (e.g. `cargo audit`, `pip-audit`, a type-checker) when `lint_proxy` can't.

### `pattern` — raw regex (DEPRECATED)

> **Deprecated. Do not reach for this first.** Top-level `strategy: pattern` is a
> blind text sweep — brittle across newlines, false-positive prone, and the source
> of past credibility gaps. `wh validate` rejects it outside a narrow allowlist.

The **only** legitimate uses are genuinely text-level checks where AST adds
nothing — string-literal *content* and naming conventions — and even those are
better expressed as regex bounded by `ast_scope` inside an `ast` signal. If you
think you need a top-level `pattern`, re-run the bucket audit: it is almost always
bucket 1 (`lint_proxy`) or bucket 3 (`ast` + `ast_query`).

| Allowlisted text-level check | Note |
|------------------------------|------|
| Naming convention (e.g. function casing) | prefer `ast_scope`-bounded |
| String-literal content (e.g. forbidden message text) | prefer `ast_scope`-bounded |

## Weight definitions

| Weight | Meaning | Usage |
|--------|---------|-------|
| `required` | Rule fails if this signal fires | Use for the primary check. A rule should have exactly one `required` signal. |
| `strong` | Significant indicator | Secondary checks that strongly support the rule. |
| `moderate` | Supporting evidence | Additional context. |

## Threshold gating

Rules with multiple signals can combine deterministic evidence. The
`deterministic_pass_threshold` / `deterministic_fail_threshold` fields require a
minimum number of fired signals before a rule counts as a violation.

```yaml
deterministic_pass_threshold: 3  # ≥3 deterministic signals = auto-pass
deterministic_fail_threshold: 0  # 0 deterministic signals = auto-fail
```

## Decomposition checklist

When decomposing a rule into signals:

- [ ] The candidate was run through the three-bucket audit (and bucket-2 items
      were moved to the skill, not forced into a signal)
- [ ] The primary signal is `ast` (with `ast_query`), `lint_proxy`, or a
      `formatter` / `tests` / `validators` binding — not a raw `pattern`
- [ ] `lint_proxy` signals include structured `lint.tool` + `lint.code`
- [ ] `ast` signals carry a real `ast_query`
- [ ] No signal is redundant with another
- [ ] Exactly one signal has weight `required`
- [ ] All signals have descriptive `description` fields
- [ ] Signal IDs are unique within the rule

## Language support matrix

| Strategy | Python | TypeScript | Rust |
|----------|--------|------------|------|
| `ast` (`ast_query`) | tree-sitter | tree-sitter | tree-sitter |
| `lint_proxy` | ruff overlay | biome config | clippy config |
| `pattern` (deprecated) | allowlist only | allowlist only | allowlist only |

`wh scan` runs tree-sitter for `ast_query` / `ast_scope` across all three
languages, regex for allowlisted `match:` checks, and lint-config verification for
`lint_proxy`.

### Supported signal patterns by language

#### Python
- Function signatures (async/sync, parameters, decorators)
- Import statements and paths
- Class inheritance and method overrides
- Keyword argument presence/absence
- String-literal content (allowlisted text-level checks)

#### TypeScript
- Import structure and deprecated API call shape (tree-sitter)
- Async/sync function form
- `any`-type and similar checks via `lint_proxy` (biome)
- Complex type-dependent checks belong in the skill (bucket 2)

#### Rust
- Import (`use`) structure and deprecated API call shape (tree-sitter)
- `.unwrap()` / `.expect()` via `lint_proxy` (clippy `unwrap_used`)
- Security/advisory dups (RUSTSEC) via `validators: command` (e.g. `cargo audit`)
  or documented as owned by that tool — not reimplemented
- Type-dependent checks (e.g. "is this a `reqwest::Client`?") belong in the skill
  (bucket 2) — tree-sitter has no type resolution
