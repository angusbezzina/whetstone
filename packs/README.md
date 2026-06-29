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
- needs **no type resolution** (tree-sitter can't do it; such conventions live as skill taste-guidance instead),
- ships 3–5 golden examples and passes `wh eval` (goldens run through the real scanner — gated in CI by `tests/corpus_packs.rs`),
- does **not** duplicate what ruff / biome / clippy already enforce.

Provenance: rules were researched from current (2026) official docs, adversarially
verified for source-fidelity/severity/non-duplication, then mechanically gated
through `wh eval`. See epic `whetstone-5ox` and `planning/skill-cli-boundary.md`.

## Available packs

| Pack | Language | Rules |
|------|----------|-------|
| `python/fastapi.yaml` | python | `fastapi.annotated-depends`, `fastapi.lifespan-over-on-event` |
| `python/httpx.yaml` | python | `httpx.use-client-not-top-level-api`, `httpx.proxies-argument-removed`, `httpx.app-argument-removed`, `httpx.verify-string-deprecated`, `httpx.cert-argument-deprecated` |
| `python/pydantic.yaml` | python | `pydantic.deprecated-config-class`, `pydantic.deprecated-parse-methods` |
| `python/sqlalchemy.yaml` | python | `sqlalchemy.declarative-base-class` |
| `rust/axum.yaml` | rust | `axum.path-param-braces` |
| `rust/serde.yaml` | rust | `serde.derive-import-from-serde` |
| `typescript/express.yaml` | typescript | `express.no-sendfile`, `express.send-status-number`, `express.no-redirect-back`, `express.no-req-param` |
| `typescript/next.yaml` | typescript | `next.revalidate-tag-requires-profile`, `next.no-legacy-image-import` |
| `typescript/react.yaml` | typescript | `react.no-reactdom-render`, `react.no-reactdom-hydrate`, `react.no-prop-types` |
| `typescript/zod.yaml` | typescript | `zod.top-level-string-formats`, `zod.no-native-enum`, `zod.record-two-args`, `zod.no-object-strict-passthrough`, `zod.no-dropped-error-params` |

## Currency (keeping packs trustworthy)

Whetstone's core thesis is that rules go stale, so the corpus must be maintained:

- Each pack's `metadata` carries `version` + `last_validated`.
- `tests/corpus_packs.rs` re-runs `wh eval` on every pack in CI, so a rule whose
  golden behaviour regresses fails the build immediately.
- **Re-validation cadence:** when a covered dependency ships a new major/minor, or at
  least quarterly, re-run the corpus-research workflow for that dep, confirm the cited
  `source_url`/`source_quote` still hold against the current docs, bump
  `last_validated` (and `version` if rules change), and re-run `wh eval`.
- Consumers get drift detection for their own copy automatically: `wh ci` flags when
  a dependency's docs change after a rule was authored (content-hash drift).
