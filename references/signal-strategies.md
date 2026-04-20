# Signal Strategies Reference

> How Whetstone decomposes rules into testable signals.

## Overview

Every Whetstone rule is decomposed into one or more **signals** — individual checks that can verify whether code follows the rule. The goal is 100% deterministic coverage: every rule must have at least one AST, pattern, or `lint_proxy` signal.

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

## Weight Definitions

| Weight | Meaning | Usage |
|--------|---------|-------|
| `required` | Rule fails if this signal fires | Use for the primary check. A rule should have exactly one `required` signal. |
| `strong` | Significant indicator | Use for secondary checks that strongly support the rule. |
| `moderate` | Supporting evidence | Use for additional context. |

## Threshold Gating

Rules with multiple signals can use threshold gating to combine deterministic
evidence. The `deterministic_pass_threshold` / `deterministic_fail_threshold`
fields let a rule require a minimum number of fired signals before it counts
as a violation.

```yaml
deterministic_pass_threshold: 3  # ≥3 deterministic signals = auto-pass
deterministic_fail_threshold: 0  # 0 deterministic signals = auto-fail
```

## Decomposition Checklist

When decomposing a rule into signals:

- [ ] At least one signal has strategy `ast` or `pattern`
- [ ] No signal is redundant with another
- [ ] Exactly one signal has weight `required`
- [ ] All signals have descriptive `description` fields
- [ ] Signal IDs are unique within the rule

## Language Support Matrix

| Strategy | Python | TypeScript | Rust |
|----------|--------|------------|------|
| `ast` | Full (`ast` stdlib) | Regex approximation | String matching |
| `pattern` | Full (regex) | Full (regex) | Full (string/regex) |
| `lint_proxy` | Ruff overlay | Biome config | Clippy config |

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
