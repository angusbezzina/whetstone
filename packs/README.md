# Whetstone starter rule packs

Trusted, high-confidence rule packs you can import into any project's
`whetstone/whetstone.yaml` via `extends`:

```yaml
version: 1
extends:
  - scope: project
    ref: path:./whetstone/packs/fastapi.yaml   # copy a pack in, or reference by path
```

Every rule in a pack:
- is backed by a specific documentation URL (`source_url`) + verbatim `source_quote`,
- uses a deterministic `strategy: ast` (tree-sitter) or `lint_proxy` signal — never bare regex,
- ships 3–5 golden examples and passes `wh eval` (goldens run through the real scanner),
- does **not** duplicate what ruff / biome / clippy already enforce.

Conventions that need type resolution (which tree-sitter can't do) are intentionally
NOT here — they live as skill taste-guidance (see `SKILL.md`), per
`planning/skill-cli-boundary.md`.

## Available packs

| Pack | Language | Rules |
|------|----------|-------|
| `python/fastapi.yaml` | Python | `fastapi.annotated-depends` |

This corpus is being built out under epic `whetstone-5ox`; coverage grows as each
pack passes the `wh eval` quality bar and the value go/no-go.
