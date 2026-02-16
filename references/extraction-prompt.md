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
