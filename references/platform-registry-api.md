# Platform Registry API Sketch

> **Status:** draft contract · 2026-05-06
> **Tracking:** whetstone-s60, whetstone-hcr

This is a reference contract for the future shared registry. It is not yet a
shipped runtime API.

## Primary object: published pack

```json
{
  "api_version": "whetstone/v1alpha1",
  "kind": "published_pack",
  "publisher": "acme",
  "name": "python-base",
  "version": "1.2.0",
  "visibility": "public",
  "metadata": {
    "title": "Acme Python Base",
    "summary": "Shared trusted-source and rule baseline for Python services",
    "languages": ["python"],
    "dependencies": ["fastapi", "pydantic"],
    "source_kinds": ["official_docs", "team_guide"]
  },
  "pack": {
    "ref": "registry://acme/python-base@1.2.0",
    "content_hash": "sha256:..."
  },
  "scorecard": {
    "adoption_count": 124,
    "approval_rate": 0.91,
    "retention_rate": 0.87,
    "false_positive_rate": 0.03
  },
  "published_at": "2026-05-06T12:00:00Z"
}
```

## Search endpoint

### Request

```http
GET /v1/packs?language=python&dependency=fastapi&limit=20
```

### Response

```json
{
  "items": [
    {
      "publisher": "acme",
      "name": "python-base",
      "version": "1.2.0",
      "summary": "Shared trusted-source and rule baseline for Python services",
      "rank": 0.94,
      "dependencies": ["fastapi", "pydantic"]
    }
  ]
}
```

## Fetch endpoint

### Request

```http
GET /v1/packs/acme/python-base/1.2.0
```

### Response

Returns the published-pack object plus the resolved pack body.

## Publish endpoint

### Request

```http
POST /v1/packs
Content-Type: application/json
```

```json
{
  "publisher": "acme",
  "name": "python-base",
  "version": "1.2.0",
  "pack_body": {
    "apiVersion": "whetstone/v1alpha1",
    "kind": "RulePack",
    "language": "python"
  }
}
```

### Publish invariants

- publisher/name/version is immutable once accepted
- pack body must validate locally before publish
- provenance fields must remain intact
- private source material must not be published accidentally

## Ranking signals

Future rank inputs may include:

- adoption count
- approval rate after import
- retention rate
- violation trend improvement
- explicit user ratings later

## Registry refs in local config

Future local config may reference published packs like:

```yaml
extends:
  - scope: org
    ref: registry://acme/python-base@^1
```

Resolution should pin to a concrete version before generation so runs remain
deterministic and auditable.
