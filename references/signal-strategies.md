# Deterministic signal strategies

This is how a rule's mechanical enforcer is chosen, the first rung of the
enforcement ladder in `planning/direction.md`. Rules that fail every bucket
below become a literal Jev question or a review, never a weak mechanical check.
The full boundary is in `planning/skill-cli-boundary.md`.

## Three-bucket audit

Every candidate safeguard belongs to one bucket:

1. A native linter or formatter already expresses it: bind and verify that tool.
2. It needs taste, semantics, or type resolution: use guidance or accountable review.
3. It is type-independent structure no native tool catches: use a tree-sitter query.

Do not turn bucket 2 into a fake deterministic rule. Do not duplicate bucket 1
with custom scanning. Prefer a handful of trusted, actionable safeguards.

Each bucket-1 or bucket-3 safeguard is a rule v2 enforcer
([schema](rulebook.schema.json)), recorded through `wh change` and run by
`wh check`.

## `ast`

A real tree-sitter S-expression in `query`, with an optional `language`.
Every node captured as `@match` is a violation, and the same source and query
always produce the same findings. A query that compiles for no language, or a
whole-repository check with no file in scope, is unknown, never a pass; there
is no fallback to raw text matching.

```json
{"kind": "ast", "language": "rust",
 "query": "((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"}
```

## `lint`

The native linter's command and the exact rule code. The check runs the
linter with its structured reporter (Ruff, Clippy or Cargo, Biome, ESLint)
and reads the code from its JSON, so a code is never matched by searching
text.

```json
{"kind": "lint", "tool": "cargo clippy --all-targets -- -D warnings", "code": "dead_code"}
```

## Formatter, test, and validator commands

Bind existing mechanical or executable safeguards directly:

- `formatter`: the formatter in check mode, for example `cargo fmt --check`.
- `test`: a test command.
- `validator`: any other checking command.

Commands run without a shell, with a bounded environment and a time limit;
a timeout, an unavailable tool or a signal is unknown rather than a pass.

## Raw patterns

Raw regex is not an enforcement strategy. Use a native-tool binding or an
AST query.

## Initial language support

| Language | AST grammar | Native lint verification |
| --- | --- | --- |
| Python | tree-sitter-python | Ruff |
| TypeScript / JavaScript | tree-sitter-typescript | Biome |
| Rust | tree-sitter-rust | Clippy |
