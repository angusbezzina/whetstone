# Formatter-backed Enforcement Contract

> **Status:** active design + implementation notes · 2026-05-06
> **Tracking:** whetstone-ng9g.12, whetstone-ng9g.12.1

## Goal

Use formatter-backed enforcement as a **narrow fourth surface** alongside agent
context, tests, and lint overlays.

## Eligibility

A rule is eligible for formatter-backed enforcement only when all of the
following are true:

1. The rule is already an **approved** Whetstone rule.
2. The rule is still **source-backed** like any other Whetstone rule.
3. The rule still has at least one **deterministic signal** (`ast`, `pattern`, or
   `lint_proxy`). Formatter output does not replace deterministic checking.
4. The enforcement maps to a **safe mechanical rewrite**, not a subjective or
   context-heavy judgment call.
5. The rule opts in explicitly with a `formatter:` block.

## Representation

Rules may declare:

```yaml
formatter:
  tool: ruff | biome | rustfmt
  options:
    <key>: <string|number|boolean>
```

## Supported behavior in the first tranche

- **Python** → emitted into `ruff.whetstone.toml`
- **TypeScript** → emitted into `biome.whetstone.json`
- **Rust** → emitted into `rustfmt.whetstone.toml`

Only a curated set of low-risk scalar options is accepted per tool. Unsupported
keys are skipped with warnings.

## Non-goals

- becoming a general formatter wrapper
- blindly mirroring every formatter option each ecosystem exposes
- encoding preferences that are not backed by a trusted source or explicit user
  rule
- replacing lint/test enforcement for semantic rules

## Conflict handling

- When multiple rules set the same formatter option to the same value, the
  generator de-duplicates it.
- When multiple rules set the same option to different values, the generator
  keeps the **later** value and emits a warning.
- Non-scalar values are ignored with warnings.

## Failure modes

- unsupported formatter tool → validation/generation warning
- unsupported option key → warning, option skipped
- invalid value shape → warning, option skipped
- no approved rules with formatter directives → no formatter overlay emitted

## Product rule

If there is any doubt about whether a rule is truly safe to enforce through a
formatter, do **not** add a formatter directive. Keep the rule in context/lint/
test space instead.
