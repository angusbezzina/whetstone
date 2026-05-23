# Language Capability Profiles, Source Packs, and Validator Adapters

> **Status:** active implementation spec · 2026-05-20
> **Tracking:** whetstone-dqk, whetstone-dqk.1, whetstone-dqk.1.1, whetstone-dqk.1.2, whetstone-dqk.2, whetstone-dqk.3, whetstone-dqk.3.1, whetstone-dqk.4, whetstone-dqk.5
> **Related:** `planning/shared-config-packs.md`, `planning/platform-registry.md`, `planning/formatter-enforcement.md`, `references/rule-schema.yaml`, `references/signal-strategies.md`, `references/handoff-schema.md`, `references/workflow-matrix.md`

## Goal

Make Whetstone support **any language and any trusted source** without requiring
every language to have full lint or AST enforcement on day one.

The product requirement is:

1. any repo should be able to ingest trusted sources,
2. derive candidate rules,
3. approve those rules,
4. generate agent context,
5. run the strongest available deterministic validation,
6. surface clear capability gaps instead of hard failure.

## Product stance

Whetstone remains:

- **local-first** — sources, rules, and validators resolve locally by default
- **approval-first** — extracted rules still require explicit approval
- **deterministic-first** — custom validation must remain programmatic and auditable
- **capability-graded** — weak support is acceptable if it is explicit and useful

## Problem statement

The current implementation is conceptually extensible but operationally locked to
Python, TypeScript, and Rust in multiple layers:

- manifest detection
- dependency parser dispatch
- rule submission validation
- source-language validation
- "all languages" expansion
- resolver routing
- pack validation
- linter / formatter / test binding validation

This makes every new language a cross-cutting refactor.

We need one shared contract for three related concerns:

1. **language capability profiles** — what Whetstone knows about a language
2. **source packs** — trusted, optionally polyglot source bundles not tied to a registry package
3. **validator adapters** — deterministic enforcement bindings for rules

## Non-goals

This spec does **not** require the first tranche to ship:

- a dynamic plugin installation ecosystem
- arbitrary remote code execution
- full AST support for every language
- automatic validator generation from prose
- a hosted registry or marketplace

## Architecture overview

Three abstractions anchor the design.

### 1. Language capability profile

Every language is represented by a capability profile with:

- canonical id (`python`, `typescript`, `rust`, `html`, `css`, `javascript`)
- aliases (`py`, `js`, `tsx`, etc.)
- manifest fingerprints
- resolver family (`pypi`, `npm`, `crates_io`, `manual`, `none`)
- file extensions / globs
- scan capabilities (`regex`, `tree_sitter`, `linter_proxy`, `test_binding`)
- generation capabilities (`context`, `lint`, `formatter`, `tests`)

The profile is the single source of truth. Callers should stop hard-coding
language sets.

### 2. Source pack

A source pack is a trusted, versioned bundle of one or more documents that can
seed rule extraction even when there is no dependency manifest or registry entry.

Examples:

- frontend guideline repos
- internal engineering handbooks
- framework best-practice guides
- design-system documentation
- `llms.txt` / `llms-full.txt` endpoints

Source packs may target one language, multiple languages, or a repository-wide
"taste" profile.

### 3. Validator adapter

A validator adapter is a deterministic execution binding that turns a rule into
findings. Adapters normalize results into a single Whetstone finding envelope.

Examples:

- regex scan
- tree-sitter query
- ESLint rule binding
- Stylelint rule binding
- formatter option binding
- linked native test
- curated local command adapter

## Capability tiers

Languages should be described by support tiers instead of a binary supported /
unsupported flag.

| Tier | Meaning | Minimum UX |
|---|---|---|
| 0 | Source only | ingest sources + extract + approve + context |
| 1 | Advisory scan | regex / linked-test / command-backed checks |
| 2 | Structural scan | parser or tree-sitter-backed checks |
| 3 | Native enforcement | linter / formatter / native test generation |

`wh status`, `wh scan`, and the TUI should eventually surface these capability
tiers explicitly.

## Generalized language support contract

### Canonical registry shape

Near-term implementation uses a built-in static registry in Rust. It should be
designed so a later external plugin system can map onto the same shape.

```rust
pub struct LanguageSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub manifests: &'static [&'static str],
    pub registry: &'static str,
}
```

Future expansion should add optional capability fields rather than creating a
second registry type.

### Invariants

1. Every user-facing language string is normalized through the registry.
2. "all" and "any" are **meta-languages**, not real language ids.
3. Manifest discovery derives from the registry, not hand-maintained arrays.
4. Registry-backed dependency resolution is routed through the language profile.
5. Existing Python / TypeScript / Rust behavior remains backward-compatible.

### Fallback behavior

If a language has no parser, linter, or generator support yet:

- extraction still works,
- rules may still use regex / linked-test / command adapters,
- context generation still works,
- lint / test generation reports explicit capability gaps.

## Source pack model

Source packs extend the trusted-source model beyond package registries.

### Pack responsibilities

A pack may provide:

- source URLs or repo-relative docs
- language scope
- source kind metadata
- extraction filters
- optional approved rules
- optional validator adapter defaults

### Source pack requirements

1. every document retains URL or path provenance
2. content hashes are captured for drift detection
3. manual / blog / internal sources remain distinguishable from official docs
4. packs can be personal, project, team, or org scoped

### Initial implementation

The first tranche does **not** need a new runtime artifact format. It can start
by enriching `sources.custom[]` and pack metadata while keeping the extraction
workflow intact.

## Validator adapter contract

Adapters are the core extensibility surface for custom enforcement.

### Initial adapter families

1. `regex`
2. `tree_sitter_query`
3. `lint_rule`
4. `formatter_option`
5. `linked_test`
6. `command`

### Normalized finding envelope

Every adapter should eventually emit a common finding shape:

```json
{
  "rule_id": "frontend.no-inline-handler",
  "language": "html",
  "engine": "tree_sitter_query",
  "file": "src/index.html",
  "line": 12,
  "column": 5,
  "message": "Inline handlers should be avoided",
  "severity": "should",
  "fixable": false
}
```

### Safety constraints

Custom adapters must be:

- local by default
- timeout-bound
- non-interactive
- deterministic for the same repo state
- explicit about missing tooling or config

### Command adapter rules

The command adapter is the most sensitive future surface. It should eventually
require:

- repo-local command or config reference
- explicit allowlist / consent model
- machine-readable output contract
- no network by default
- clear failure semantics (`tool_missing`, `timeout`, `invalid_output`)

## CLI and artifact impacts

### Commands that should become capability-aware

- `wh init`
- `wh extract`
- `wh extract submit`
- `wh sources add|edit|list|verify`
- `wh rules add`
- `wh actions lint`
- `wh actions test`
- `wh scan`
- `wh status`

### Short-term behavior changes

The first implementation slice should only do safe refactors:

- centralize language ids / aliases / manifest names
- derive validation lists from one registry
- canonicalize alias input (`js` -> `typescript`)
- route dependency resolution through registry metadata

### Future artifact changes

Later phases may introduce:

- validator adapter config under rules
- source-pack metadata in state artifacts
- capability reports in `status` and `scan`
- per-language or per-scope capability summaries in generated context

## Rollout plan

### Phase 1 — Central language registry

Replace duplicated language constants with a shared registry.

**Implementation focus now:**

- `src/types.rs`
- `src/detect/mod.rs`
- `src/detect/walk.rs`
- `src/resolve/mod.rs`
- `src/source_mgmt.rs`
- `src/rule_authoring.rs`
- `src/config_packs.rs`
- `src/extract.rs`
- `src/rules.rs`

### Phase 2 — Source-pack workflow

Add first-class support for trusted polyglot sources not derived from package
manifests.

### Phase 3 — Validator adapter contract

Promote current signals + formatter + tests into a more explicit adapter model
while maintaining backward compatibility with existing rule YAML.

### Phase 4 — First web-stack bundle

Ship HTML, CSS, and JavaScript as the first generalized-language proof point.

Minimum acceptable experience:

- trusted-source ingestion
- rule extraction
- approval flow
- context generation
- `scan` findings from the strongest available adapters

## Acceptance criteria

This implementation-spec pass is complete when:

1. a planning doc exists with contracts, rollout order, and safety rules
2. Beads issues map the design into implementable slices
3. the codebase begins moving from hard-coded language lists to a shared registry

## Open questions

1. Should `javascript` become a first-class language id distinct from
   `typescript`, or remain an alias until separate capabilities exist?
2. Do source packs deserve a new file format immediately, or can enriched config
   plus state artifacts carry phase 1?
3. Should custom command adapters be project-only at first, or also allowed in
   personal scope?

## Immediate implementation notes

The current tranche should stay intentionally narrow:

- **do** centralize language normalization and manifest routing
- **do** preserve backward compatibility for existing projects
- **do not** attempt dynamic plugin loading yet
- **do not** expand TUI language UX in the same change unless required

That gives Whetstone a stable substrate for the broader rollout without stalling
on the full end-state architecture.
