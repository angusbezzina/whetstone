# Extraction Prompt Reference

> This is the detailed reference for Whetstone's rule extraction prompt. The SKILL.md contains the working version; this document provides rationale and examples.

## Overview

The extraction prompt is what the agent uses to derive coding rules from dependency documentation. It's the core differentiator of Whetstone — the prompt enforces "high confidence or silence" and produces structured rule YAML.

## Prompt Structure

The prompt has six sections:

1. **Task framing** — what the agent is doing, for which dependency, with today's date and release metadata
2. **Recency priority** — explicit instruction to focus on post-training-cutoff content (last 18 months)
3. **Category definitions** — the five valid rule categories
4. **Hard filters** — absolute requirements that reject bad rules
5. **Signal decomposition** — how to break rules into testable checks
6. **Output format** — the exact YAML structure to produce

## Recency Priority

LLMs are trained on documentation snapshots typically 1-2 years old. Whetstone's highest value is catching things the LLM doesn't already know. The extraction prompt includes:

- **Today's date** — so the agent knows the current temporal context
- **Latest version and release date** — from the source-resolution / handoff output
- **Explicit prioritization** — rules about changes from the last 18 months rank highest

This means a migration footgun from 6 months ago is more valuable than a long-standing convention that every developer (and LLM) already knows. The ranking criteria put recency first.

## Hard Filters (Rejection Criteria)

These are non-negotiable. A proposed rule that violates ANY of these is rejected:

### 1. Confidence Threshold (90%+)

The documentation must clearly state or strongly imply the practice. "Best practice" blog posts don't count. The source must be official documentation, a migration guide, or a changelog.

**Passes**: "FastAPI documentation explicitly states route handlers should be async"
**Fails**: "I've seen several blog posts recommend async handlers"

### 2. Signal Requirement

Every CLI rule MUST have a deterministic backing: a `strategy: ast` signal (with a real `ast_query`) or a `strategy: lint_proxy` signal, or an explicit `formatter` / `tests` / `validators` binding when the rule is best enforced mechanically, through an existing test, or by an external tool. `strategy: pattern` (raw regex) is deprecated — the only allowed form is regex bounded by `ast_scope` inside an `ast` signal.

Guidance that can't get a deterministic signal because it needs **taste or type resolution** is not rejected outright — it moves to the skill as agent guidance (tree-sitter has no type resolution, so it can't, for example, confirm a receiver is a `reqwest::Client`). It just doesn't become a signal-less rule.

**Passes**: Rule with an `ast` signal (and `ast_query`) that distinguishes a sync from an async function form
**Becomes skill guidance, not a CLI rule**: "set a reqwest timeout" — verifying it requires knowing the receiver's type and detecting an absent `.timeout()` call

### 3. Count Ceiling (Max 5)

Maximum 5 rules per dependency. This forces prioritization. If you can identify 20 potential rules, the ceiling forces you to keep only the 5 most valuable. Ranking criteria:

1. Frequency of mistake — how often developers get this wrong
2. Severity of consequence — what happens when they do
3. Detectability — can it be caught with deterministic signals?
4. Novelty — is this already caught by standard tooling?

### 4. Novelty Requirement

Do NOT propose rules that standard linters already enforce:
- **Python**: ruff with default rules
- **TypeScript**: biome with default rules
- **Rust**: clippy with default lints

If ruff already catches unused imports, don't propose a rule for unused imports. Whetstone catches what linters miss.

### 5. Source Backing

Every rule must cite a specific URL — not just "the FastAPI docs" but `https://fastapi.tiangolo.com/async/#in-a-hurry`. The URL must be navigable and contain the relevant information.

## Category Definitions

### migration
Deprecated APIs that still work in the current version but are removed or replaced in newer versions. These are the highest-value rules because the code compiles and runs but is using the wrong API.

**Example**: Pydantic v2 — `.schema()` still works but is deprecated in favor of `model_json_schema()`.

### default
Configuration or patterns that work but are insecure, slow, or incorrect unless explicitly configured. The "it works on my machine" category.

**Example**: SQLAlchemy `echo=True` left in production code, Django `DEBUG=True`, missing CORS configuration.

### convention
Patterns where the official documentation recommends one approach but most tutorials, LLMs, and developers default to another. The gap between "what the docs say" and "what people do."

**Example**: FastAPI async route handlers — docs clearly recommend async, but most tutorials and LLM outputs use sync.

### breaking-change
Patterns that work in the current version but will break in the next major version. Proactive migration rules.

**Example**: Next.js 15 requires `async` for page components that access `params` — was synchronous in Next.js 14.

### semantic
Practices that require some judgment to enforce but can be decomposed so that the enforceable part is deterministic. The key requirement is that the rule's primary signal is deterministic (`ast` with `ast_query`, or `lint_proxy`); the residual judgment that can't be made deterministic stays in the skill as guidance rather than becoming a signal.

**Example**: "Error messages SHOULD be actionable" — the deterministic part (uses dynamic string formatting, references a variable, `ast_scope`-bounded expectation-language check) becomes the signal; "actually suggests a useful remediation" is judgment the skill applies, not a signal.

## Multi-Section Content

When the resolve pipeline provides multiple sections per dependency (e.g., README + changelog), extract from each section with different priorities:

### Changelog Sections
- **Highest signal for**: `migration`, `breaking-change` categories
- Look for: deprecated APIs, removed features, required migration steps, new defaults
- These are the most valuable rules because they represent recent changes LLMs may not know about
- Set `source_kind: changelog` on rules derived primarily from changelog evidence

### README / Documentation Sections
- **Highest signal for**: `convention`, `default` categories
- Look for: recommended patterns, configuration best practices, common pitfalls
- Set `source_kind: official_docs` for vendor documentation

### Cross-Referencing
- A changelog deprecation confirmed by README guidance → high confidence
- A README convention that contradicts the changelog's direction → investigate, may be stale
- Multiple sections agreeing on a pattern → stronger evidence

### Source Kind Attribution
Every proposed rule MUST include a `source_kind` field indicating what kind of source provided the primary evidence. This enables filtering (e.g., "show me only changelog-derived rules") and trust assessment.

Common values: `official_docs`, `changelog`, `migration_guide`, `blog`, `social`, `community`, `team_guide`, `manual`. Any string is valid — use what best describes the source.

---

## Writing enforceable signals

Lead with `ast_query` and `lint_proxy`; raw regex is the last resort.

- **Linter already catches it (bucket 1)** → `strategy: lint_proxy` with
  `lint: {tool, code}`. Don't reimplement it as regex. The bare-`.unwrap()` case,
  for instance, is clippy `unwrap_used` — bind to it, don't hand-roll `\.unwrap\(\)`.

  ```yaml
  signals:
    - id: bare-unwrap
      strategy: lint_proxy
      description: "Covered by clippy unwrap_used"
      weight: required
      lint:
        tool: clippy
        code: unwrap_used
  ```

- **Structural and type-independent (bucket 3)** → `strategy: ast` with a real
  `ast_query` (tree-sitter S-expression). Absence/negation and text refinements use
  `#match?` / `#not-match?` predicates or `ast_scope`-bounded regex *inside* the
  signal — regex scoped to a parsed node, not a blind text sweep.

  ```yaml
  signals:
    - id: sync-route-handler
      strategy: ast
      description: "Route handler declared with sync def"
      weight: required
      ast_query: |
        (function_definition name: (identifier)) @fn
  ```

- **Raw `strategy: pattern` is deprecated.** `wh validate` rejects a top-level
  `match:` regex outside a narrow allowlist (string-literal content, naming). If
  you reach for it, re-run the bucket audit first.

- **Needs a value's type (bucket 2)** → don't write a signal. Move it to the skill
  as guidance; tree-sitter cannot resolve types.

---

## Signal Decomposition Guide

Every rule is a spectrum of signals. The goal is maximum deterministic coverage; whatever can't be made deterministic stays in the skill as guidance, not as a signal.

See [signal-strategies.md](signal-strategies.md) for the three-bucket signal audit and detailed strategy descriptions.

### Decomposition Process

1. State the rule in plain language
2. Ask: "What would I look for in code to verify this?"
3. For each check, run the bucket audit: linter already catches it → `lint_proxy`; type-independent and structural → `ast` + `ast_query`; needs taste or a value's type → skill guidance (no signal)
4. If nothing deterministic survives, the rule is bucket 2 — carry it as skill guidance, don't submit a signal-less rule
5. Assign weights: the most reliable signal is `required`, supporting signals are `strong` or `moderate`

### Structured lint bindings

When using `strategy: lint_proxy`, always include structured lint metadata instead of burying it in prose:

```yaml
signals:
  - id: mutable-defaults
    strategy: lint_proxy
    description: "Covered by Ruff"
    weight: required
    lint:
      tool: ruff
      code: B006
```

This is the preferred format for agent-authored candidate bundles.

### Example Decomposition

**Rule**: "Error messages SHOULD be actionable"

| Check | Disposition | Weight |
|-------|-------------|--------|
| Uses dynamic string formatting | `ast` (`ast_query`) | required |
| References a variable from scope | `ast_scope`-bounded check inside the signal | strong |
| Contains expectation language | `ast_scope`-bounded string-content check | moderate |
| "Actually suggests a useful remediation" | skill guidance — judgment, not a signal | — |

Result: the deterministic checks become the rule's signal; the residual judgment stays in the skill rather than becoming a non-deterministic signal.

## Golden Examples

Every rule requires 3-5 golden examples — code snippets with known pass/fail verdicts. These serve three purposes:

1. **Test generation** — examples become the basis for generated test files
2. **Signal calibration** — examples are the known-answer set the rule's `ast` / `lint_proxy` signal must agree with; if the scanner disagrees with a golden example, the signal is wrong
3. **Agent grounding** — examples ground the skill's judgment when it applies the rule (and bucket-2 taste guidance) mid-turn

### Writing Good Examples

- Include realistic, production-like code (not toy examples)
- Cover edge cases, not just obvious pass/fail
- Include at least one "close call" example
- Provide a `reason` field explaining the verdict
- Use the actual APIs and patterns from the dependency

## Candidate Rule Format

When the extraction prompt produces rules, they are initially in **candidate** status. The candidate format differs from the final approved format in these ways:

| Field | Candidate | Approved |
|-------|-----------|----------|
| `status` | `candidate` | `approved` |
| `approved` | `false` | `true` |
| lifecycle metadata | absent | absent in the lean schema |

### Candidate Artifacts

Candidate rules are stored in `whetstone/rules/{language}/{dependency}.yaml` with `status: candidate`. They remain there until the user reviews them.

### Lifecycle Transitions

```
candidate → approved   (user approves explicitly)
```

Rules that should not continue to exist are removed from the ruleset rather than
transitioned to extra lifecycle states.

## Stale Rule Detection

Whetstone detects rule staleness through two mechanisms:

### 1. Content Hash Drift

Each rule's source has a `content_hash` (SHA-256 of the fetched documentation content). When `wh reinit` re-fetches documentation:

- If the hash matches: source is unchanged, rules are **current**
- If the hash differs: source has changed, rules are **stale** and should be re-evaluated

The refresh flow uses this mechanism to identify which dependencies need re-extraction.

### 2. Version Drift

When a dependency's version in the manifest differs from the version recorded in the rule YAML's `source.version`, that's version drift. `wh reinit --check` identifies these.

### 3. Time-based Freshness

Rules older than 60 days are flagged as potentially stale regardless of hash/version, since documentation may have been updated between major releases.

### Validation Workflow

When drift is detected:
1. Re-resolve the source (`wh reinit`)
2. Review `refresh-diff.json` and `re_extraction_candidates`
3. Re-author or update the affected candidate/approved rules
4. Regenerate outputs with `wh actions all`
