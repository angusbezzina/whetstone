# `wh config conflicts --json` output (whetstone-l05)

Cross-layer rule conflicts, surfaced so the onboarding CONFLICTS step (and agents)
can resolve them deterministically. Pass `--with-pack <file>` (repeatable) to also
include conflicts a *proposed* pack selection would introduce, before importing it.

```json
{
  "status": "ok",
  "conflicts_count": 2,
  "conflicts": [ /* Conflict */ ]
}
```

## Conflict variants

### `same-id`
Two or more sources define the same rule id. Sources are packs (`pack:<name>`),
`project-local` (whetstone/rules/), and `personal` (.personal/rules/). The
`winner` is chosen by the merge's precedence — personal > project-local > later
pack > earlier pack — and is the definition that actually applies.

```json
{
  "kind": "same-id",
  "rule_id": "fastapi.annotated-depends",
  "winner": "pack:rival.pack",
  "losers": ["pack:whetstone.fastapi"],
  "layers": ["pack:whetstone.fastapi", "pack:rival.pack"],
  "suggested_resolution": "the winning layer applies; add a `deny` or `override` entry for the others if the shadow is unintended"
}
```

### `formatter-option`
Two active rules bind the same formatter `tool` option to different values, so the
generated linter/formatter config would be inconsistent.

```json
{
  "kind": "formatter-option",
  "tool": "ruff",
  "option": "line-length",
  "values": [
    { "rule_id": "fmt.a", "value": 88 },
    { "rule_id": "fmt.b", "value": 100 }
  ],
  "suggested_resolution": "override one rule's formatter option so the generated tool config is consistent"
}
```

Resolution is written as `deny` / `overrides` entries in `whetstone.yaml` (or the
personal config) — the same mechanism the wizard's CONFLICTS step uses.
