# Signal Strategies Reference

> How Whetstone decomposes rules into testable signals.

## Overview

Every Whetstone rule is decomposed into one or more **signals** — individual checks that can verify whether code follows the rule. The goal is to maximize deterministic coverage: catch as much as possible with AST and pattern checks before resorting to AI judgment.

## Strategy Types

### `ast` — Syntax Tree Analysis

**What it does**: Parses source code into an Abstract Syntax Tree and checks structural properties.

**When to use**:
- Function signatures (async vs sync, parameter types, return types)
- Decorator presence or absence
- Class inheritance patterns
- Import statements and their structure
- Control flow patterns (nesting depth, early returns)
- Method calls on specific objects

**Deterministic**: Yes — always produces the same result for the same code.

**Implementation**:
- Python: `ast` stdlib module
- TypeScript: regex-based (MVP) or TypeScript compiler API
- Rust: string/regex matching (MVP) or `syn` crate

**Examples**:

| Check | AST Node Type |
|-------|---------------|
| Function is async | `AsyncFunctionDef` vs `FunctionDef` |
| Has decorator X | `FunctionDef.decorator_list` |
| Imports from module | `ImportFrom.module` |
| Class inherits from X | `ClassDef.bases` |
| Nesting depth > N | Recursive `If`/`For`/`While` count |
| Calls deprecated method | `Call.func.attr` |

### `pattern` — Text/Regex Matching

**What it does**: Searches source code text using regular expressions or string matching.

**When to use**:
- String literals (error messages, config values)
- Naming conventions (variable names, file names)
- Comment patterns (TODO, FIXME, specific annotations)
- Import ordering or grouping
- Configuration values in code
- Version strings or magic numbers

**Deterministic**: Yes.

**Implementation**: Standard regex across all languages.

**Examples**:

| Check | Pattern |
|-------|---------|
| Uses deprecated `.schema()` | `\.schema\(\)` |
| Has hardcoded secret | `(password\|secret\|api_key)\s*=\s*["']` |
| Uses old import path | `from old_module import` |
| Missing type annotation | `def \w+\([^:)]+\)` (no colon in params) |

### `lint_proxy` — Existing Linter Rule

**What it does**: Maps to an existing rule in ruff, biome, or clippy that isn't enabled by default.

**When to use**:
- When a linter has a rule for this check but it's not in the default set
- When the linter rule needs specific configuration
- When combining multiple linter rules covers the check

**Deterministic**: Yes (delegated to the linter).

**Implementation**: Generates linter configuration overlays:
- Python: `ruff.whetstone.toml` with `extend-select`
- TypeScript: `biome.whetstone.json` with rule config
- Rust: `clippy.whetstone.toml` with lint settings

**Examples**:

| Check | Linter Rule |
|-------|-------------|
| Unused function arguments | ruff: `ARG001` |
| Mutable default arguments | ruff: `B006` |
| Use of `any` type | biome: `noExplicitAny` |
| Unwrap without expect | clippy: `unwrap_used` |

### `ai` — LLM Judgment

**What it does**: Sends code to an LLM with a narrow binary question and few-shot examples.

**When to use**: ONLY when deterministic signals cannot cover the check. Common cases:
- Semantic quality (is this error message helpful?)
- Intent verification (does this function name match its behavior?)
- Contextual correctness (is this the right abstraction for this use case?)

**Deterministic**: No — results may vary between runs and models.

**Implementation**: AI eval definitions with:
- A specific binary question (PASS or FAIL)
- 2-3 golden examples as few-shot grounding
- An AST-based pre-filter that selects which code to evaluate
- A one-line reason requirement

**Important constraints**:
- AI signals can NEVER be the only signal in a rule
- AI signals should be `moderate` weight, not `required`
- Every AI signal needs golden examples for calibration
- AI eval runs only on ambiguous cases (between pass and fail thresholds)

## Weight Definitions

| Weight | Meaning | Usage |
|--------|---------|-------|
| `required` | Rule fails if this signal fires | Use for the primary check. A rule should have exactly one `required` signal. |
| `strong` | Significant indicator | Use for secondary checks that strongly support the rule. |
| `moderate` | Supporting evidence | Use for additional context. AI signals should be `moderate`. |

## Threshold Gating

Rules with multiple signals use threshold gating to minimize AI usage:

1. **All deterministic signals present** → Auto-pass, no AI needed
2. **Zero deterministic signals present** → Auto-fail, no AI needed
3. **In between** → Ambiguous, send to AI for judgment

Configure thresholds per rule:
```yaml
deterministic_pass_threshold: 3  # ≥3 deterministic signals = auto-pass
deterministic_fail_threshold: 0  # 0 deterministic signals = auto-fail
```

This means AI eval costs scale with ambiguity, not codebase size.

## Decomposition Checklist

When decomposing a rule into signals:

- [ ] At least one signal has strategy `ast` or `pattern`
- [ ] No signal is redundant with another
- [ ] Exactly one signal has weight `required`
- [ ] AI signals (if any) have weight `moderate`
- [ ] AI signals have a clear binary question
- [ ] All signals have descriptive `description` fields
- [ ] Signal IDs are unique within the rule

## Language Support Matrix

| Strategy | Python | TypeScript | Rust |
|----------|--------|------------|------|
| `ast` | Full (`ast` stdlib) | Regex approximation | String matching |
| `pattern` | Full (regex) | Full (regex) | Full (string/regex) |
| `lint_proxy` | Ruff overlay | Biome config | Clippy config |
| `ai` | Supplement only | Supplement only | Supplement only |

### Supported Signal Patterns by Language

#### Python (Reference Implementation)
- Function signatures (async/sync, parameters, decorators)
- Import statements and paths
- Class inheritance and method overrides
- Keyword argument presence/absence
- String literal patterns
- Deprecated API calls

#### TypeScript (Baseline)
- Deprecated API calls (pattern matching)
- Import statement checks (pattern matching)
- String literal checks (pattern matching)
- Async/sync function detection (regex approximation)
- Complex AST checks generate TODO scaffolds

#### Rust (Baseline)
- Deprecated API calls (string contains)
- Unsafe block detection
- .unwrap() usage detection
- use statement checks
- Complex AST checks generate TODO scaffolds with Dylint reference
