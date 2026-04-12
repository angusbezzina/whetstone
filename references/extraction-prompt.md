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
- **Latest version and release date** — from the resolve-sources output
- **Explicit prioritization** — rules about changes from the last 18 months rank highest

This means a migration footgun from 6 months ago is more valuable than a long-standing convention that every developer (and LLM) already knows. The ranking criteria put recency first.

## Hard Filters (Rejection Criteria)

These are non-negotiable. A proposed rule that violates ANY of these is rejected:

### 1. Confidence Threshold (90%+)

The documentation must clearly state or strongly imply the practice. "Best practice" blog posts don't count. The source must be official documentation, a migration guide, or a changelog.

**Passes**: "FastAPI documentation explicitly states route handlers should be async"
**Fails**: "I've seen several blog posts recommend async handlers"

### 2. Signal Requirement

Every rule MUST have at least one deterministic signal with strategy `ast` or `pattern`. This ensures the rule can be checked without AI. Pure `ai`-only rules are rejected because they can't be reliably enforced in CI.

**Passes**: Rule with an `ast` signal that checks for `FunctionDef` vs `AsyncFunctionDef`
**Fails**: Rule with only an `ai` signal that says "check if the function should be async"

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
Practices that require some judgment to enforce but can be decomposed into mostly-deterministic signals. The key requirement is that at least one signal must be deterministic.

**Example**: "Error messages SHOULD be actionable" — decomposed into: uses f-string (ast), references a variable (pattern), contains expectation language (pattern), suggests remediation (ai).

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

## Match Patterns for Signals

Every `pattern`-strategy signal SHOULD include a `match` field with a concrete regex pattern. This is critical: without `match`, generated tests produce TODO stubs that check nothing. With `match`, generated tests contain real regex checks that catch violations in CI.

```yaml
signals:
  - id: bare-unwrap
    strategy: pattern
    description: "Detects .unwrap() calls"
    match: '\.unwrap\s*\(\)'     # Concrete regex — enables real tests
    weight: required
```

**Guidelines for writing match patterns:**
- Use standard regex syntax (Rust `regex` crate, Python `re` module)
- Keep patterns simple — one pattern per signal, not compound regex
- Test the pattern mentally against the golden examples
- For complex checks that need multi-line or AST awareness, use `strategy: ast` without `match` (deferred to tree-sitter)

---

## Signal Decomposition Guide

Every rule is a spectrum of signals. The goal is to maximize deterministic coverage before resorting to AI.

See [signal-strategies.md](signal-strategies.md) for detailed strategy descriptions and examples.

### Decomposition Process

1. State the rule in plain language
2. Ask: "What would I look for in code to verify this?"
3. For each check, determine if it can be done via AST, regex, or linter rule
4. Only use AI for what's left
5. Assign weights: the most reliable signal is `required`, supporting signals are `strong` or `moderate`

### Example Decomposition

**Rule**: "Error messages SHOULD be actionable"

| Signal | Strategy | Weight | Deterministic? |
|--------|----------|--------|----------------|
| Uses dynamic string formatting | ast | required | Yes |
| References a variable from scope | pattern | strong | Yes |
| Contains expectation language | pattern | moderate | Yes |
| Suggests a remediation | ai | moderate | No |

Result: 3 of 4 signals are deterministic. The AI signal is only needed for ambiguous cases.

## Golden Examples

Every rule requires 3-5 golden examples — code snippets with known pass/fail verdicts. These serve three purposes:

1. **Test generation** — examples become the basis for generated test files
2. **Prompt grounding** — examples ground AI eval prompts in known answers
3. **Calibration** — if AI eval disagrees with golden examples, the prompt needs fixing

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
| `approved_at` | absent | ISO 8601 timestamp |
| `proposed_at` | ISO 8601 timestamp | preserved from candidate |
| `proposed_by` | `"whetstone-extraction"` | preserved from candidate |

### Candidate Artifacts

Candidate rules are stored in `whetstone/rules/{language}/{dependency}.yaml` with `status: candidate`. They remain there until the user reviews them.

### Lifecycle Transitions

```
candidate → approved   (user approves during review)
candidate → denied     (user rejects with optional reason)
approved  → deprecated (rule is superseded or source becomes invalid)
```

Denied rules are kept in the YAML file with `status: denied` so the same rule isn't re-proposed on the next extraction run. The `denied_reason` field captures why, so future extraction can respect the decision.

## Stale Rule Detection

Whetstone detects rule staleness through two mechanisms:

### 1. Content Hash Drift

Each rule's source has a `content_hash` (SHA-256 of the fetched documentation content). When `resolve-sources.py` re-fetches documentation:

- If the hash matches: source is unchanged, rules are **current**
- If the hash differs: source has changed, rules are **stale** and should be re-evaluated

The `--changed-only` flag on `resolve-sources.py` and `detect-deps.py` uses this mechanism to identify which dependencies need re-extraction.

### 2. Version Drift

When a dependency's version in the manifest differs from the version recorded in the rule YAML's `source.version`, that's version drift. `detect-deps.py --check-drift` identifies these.

### 3. Time-based Freshness

Rules older than 60 days are flagged as potentially stale regardless of hash/version, since documentation may have been updated between major releases.

### Validation Workflow

When drift is detected:
1. Re-resolve the source (`resolve-sources.py --changed-only`)
2. Compare new content against existing rules
3. Propose updates, additions, or deprecations
4. Mark validated rules with `last_validated` and `validation_status: current`
