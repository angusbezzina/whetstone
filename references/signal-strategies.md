# Deterministic signal strategies

This is the temporary R0 kernel contract. It preserves a real safety bar while
Direction 05's typed agreement records are built. The full judgment boundary is
in `planning/skill-cli-boundary.md`.

## Three-bucket audit

Every candidate safeguard belongs to one bucket:

1. A native linter or formatter already expresses it: bind and verify that tool.
2. It needs taste, semantics, or type resolution: use guidance or accountable review.
3. It is type-independent structure no native tool catches: use a tree-sitter query.

Do not turn bucket 2 into a fake deterministic rule. Do not duplicate bucket 1
with custom scanning. Prefer a handful of trusted, actionable safeguards.

## `ast`

An AST signal contains a real tree-sitter S-expression in `ast_query`. Every
node captured as `@match` is a violation. The same source and query must always
produce the same findings.

```yaml
signals:
  - id: sync-route-handler
    strategy: ast
    description: Route handler is synchronous
    weight: required
    ast_query: '(function_definition) @match'
```

The R0 validator rejects project AST signals without a query. The scanner never
falls back to raw text matching if parsing or query compilation fails.

## `lint_proxy`

A lint proxy declares the native tool and exact rule code. The kernel verifies
that project configuration actually enables it; it does not pretend to run or
generate a second implementation.

```yaml
signals:
  - id: mutable-defaults
    strategy: lint_proxy
    description: Ruff owns this safeguard
    weight: required
    lint:
      tool: ruff
      code: B006
```

R0 supports Ruff, Biome, and Clippy configuration checks.

## Formatter, test, and validator bindings

Bind existing mechanical or executable safeguards directly:

- `formatter`: Ruff, Biome, or rustfmt configuration.
- `tests`: pytest, Vitest, or Cargo test paths and optional selectors.
- `validators`: an explicit command or linked/native check.

Executable validators are untrusted. Paths must be repository-relative,
execution is time-bounded, and errors or unavailable tools are configuration
issues rather than passes.

## `pattern`

Raw regex patterns are legacy migration data, not executable R0 enforcement.
The scanner skips them explicitly. New project rules must use native-tool
bindings or AST queries. Personal legacy records may remain readable so their
owners can migrate them later, but they cannot be represented as enforced.

## Golden examples

Every approved scanner-backed rule needs both known-good and known-bad examples.
`wh eval` passes each example through the real scanner. A mismatch fails the
gate; a rule with no scanner-backed examples does not count toward non-vacuity.

## Initial language support

| Language | AST grammar | Native lint verification |
| --- | --- | --- |
| Python | tree-sitter-python | Ruff |
| TypeScript / JavaScript | tree-sitter-typescript | Biome |
| Rust | tree-sitter-rust | Clippy |
