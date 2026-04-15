# Whetstone Handoff Schema

> The contract between the Whetstone binary (deterministic) and the agent
> (judgment). All handoff artifacts live under `whetstone/.state/` and are JSON
> with a top-level `version: 1` field for forward compatibility.

These artifacts are **agent-readable** and **user-reviewable**. They exist so
that extraction, refresh, and eval workflows are reproducible and resumable —
an agent can crash mid-run and a fresh agent can resume from the file rather
than re-doing the work.

---

## Extraction handoff

**File:** `whetstone/.state/extraction-handoff.json`
**Writer:** `wh doctor`, `wh refresh` (always rewrite on run)
**Reader:** agent (extraction), user (review)

Produced whenever a doctor or refresh run finishes resolving sources and has
dependencies that still need rule extraction.

### Shape

```json
{
  "version": 1,
  "generated_at": "2026-04-13T12:34:56Z",
  "trigger": "doctor | refresh",
  "project_dir": "/path/to/project",
  "languages": ["python", "rust", "typescript"],
  "candidates": [
    {
      "name": "reqwest",
      "language": "rust",
      "version": "0.12.0",
      "source_type": "readme | llms_txt | llms_full_txt | html_converted | changelog | docs_url_only",
      "source_url": "https://docs.rs/reqwest",
      "content_hash": "sha256:...",
      "sections": [
        { "type": "readme", "url": "...", "bytes": 12345 },
        { "type": "changelog", "url": "...", "bytes": 4567, "versions_covered": "0.11.20–0.12.0" }
      ],
      "existing_rules": 0,
      "priority": "ready_now | resolved_low | pending | failed",
      "reason": "optional: why this bucket"
    }
  ],
  "skipped": [
    { "name": "foo", "reason": "already has approved rules; source unchanged" }
  ],
  "next_action": "Apply extraction prompt to each candidate; approve or deny; then wh validate && wh context && wh tests"
}
```

### Lifecycle notes

- `candidates` are ordered highest-priority-first (bucketed by `priority`).
- `priority: ready_now` — source is `llms_txt` / `llms_full_txt` with high confidence.
- `priority: resolved_low` — source is readme / html_converted / docs_url_only.
- `priority: pending` — source hasn't been resolved yet (missing from cache).
- `priority: failed` — resolution failed this run; `reason` explains why.
- The agent MUST re-read `extraction-handoff.json` on resume, not the doctor
  output (the doctor output is ephemeral but this file is durable).

### `worklist` (since 3D.1.2)

The same artifact also carries a `worklist` array — a richer, per-dependency
view of the same work that `candidates` describes flat. Each entry bundles the
dep's ranked sources, section summaries, quota (from `extraction.max_rules_per_dep`),
and a concrete next-step hint. The worklist is sorted by priority bucket then
by a configuration-aware score (`preferred_source_kinds`, `recency_window_days`).

```json
"worklist": [
  {
    "name": "fastapi",
    "language": "python",
    "priority": "ready_now",
    "score": 132.0,
    "version": "0.110",
    "source_type": "llms_full_txt",
    "source_url": "https://fastapi.tiangolo.com/llms-full.txt",
    "content_hash": "sha256:...",
    "freshness": { "confidence": "high" },
    "sections": [
      { "type": "readme", "url": "...", "bytes": 12345 }
    ],
    "existing_rules": 1,
    "quota": { "max_rules_per_dep": 5, "remaining": 4 },
    "next_step": "Read the linked source, propose up to 4 rule(s), then `wh propose import <bundle>`",
    "allowed_categories": ["migration", "convention"],
    "min_confidence": "high",
    "preferred_source_kinds": ["changelog", "migration_guide"]
  }
]
```

Access via `wh review worklist [--dep=<name>] [--lang=<python|typescript|rust>]`.
Older readers that don't know the key ignore it; the field is fully additive.

---

## Refresh diff

**File:** `whetstone/.state/refresh-diff.json`
**Writer:** `wh refresh` (only; always rewrites)
**Reader:** agent (focused re-extraction), user (review)

Produced every time `wh refresh` runs. Captures **what changed** between the
previous cache and the current resolution, so the agent can re-extract only
against the delta instead of re-reading every source.

### Shape

```json
{
  "version": 1,
  "generated_at": "2026-04-13T12:34:56Z",
  "project_dir": "/path/to/project",
  "drift_count": 2,
  "changed": [
    {
      "name": "pydantic",
      "language": "python",
      "previous_version": null,
      "current_version": "2.7.0",
      "previous_content_hash": null,
      "current_content_hash": "sha256:bbb",
      "changed_sections": ["changelog", "readme"],
      "affected_rule_ids": ["pydantic.schema-method", "pydantic.validate-assignment"],
      "source_urls": {
        "docs": "https://docs.pydantic.dev/latest/",
        "changelog": "https://raw.githubusercontent.com/pydantic/pydantic/main/HISTORY.md"
      }
    }
  ],
  "unchanged_with_stale_cache": [
    { "name": "fastapi", "language": "python", "reason": "cache TTL expired; content unchanged" }
  ],
  "removed": [
    { "name": "six", "language": "python", "reason": "dropped from manifest" }
  ],
  "failed": [
    { "name": "obscurelib", "language": "python", "error": "HTTP 404" }
  ],
  "next_action": "For each changed dep, re-read its new content and propose: new rules, modified rules, rules to deprecate (status: deprecated)."
}
```

### Lifecycle notes

- `drift_count` is authoritative for `wh refresh --check` gating (non-zero
  drift = exit 1).
- `affected_rule_ids` lists approved rules that cite the changed dep's source;
  the agent should review each for possible deprecation.
- `previous_version` and `previous_content_hash` are **optional** — the
  shipped writer leaves them `null` because the cache is overwritten during
  refresh before the diff is assembled. A future enhancement can snapshot the
  pre-refresh cache; readers MUST tolerate `null` in these fields.

---

## Eval requests

**File:** `whetstone/.state/eval-requests.json`
**Writer:** `wh eval run` (when any rule with `ai_eval` fires on a non-deterministic case)
**Reader:** agent (judgment)

### Shape

```json
{
  "version": 1,
  "generated_at": "2026-04-13T12:34:56Z",
  "instructions": "For each request, answer PASS or FAIL with a one-line reason. Write to eval-verdicts.json.",
  "response_format": {
    "version": 1,
    "verdicts": [{ "id": "...", "verdict": "pass|fail", "reason": "..." }]
  },
  "requests": [
    {
      "id": "reqwest.set-timeout:src/client.rs:42",
      "rule_id": "reqwest.set-timeout",
      "question": "Does this client have an explicit timeout?",
      "code_snippet": "let c = Client::new();",
      "file_path": "src/client.rs",
      "line_start": 38,
      "line_end": 48,
      "golden_examples": [
        { "code": "...", "verdict": "pass", "reason": "..." }
      ]
    }
  ]
}
```

## Eval verdicts

**File:** `whetstone/.state/eval-verdicts.json`
**Writer:** agent
**Reader:** `wh eval run --collect`

### Shape

```json
{
  "version": 1,
  "judged_at": "2026-04-13T12:45:00Z",
  "verdicts": [
    { "id": "reqwest.set-timeout:src/client.rs:42", "verdict": "fail", "reason": "Client::new() has no timeout" }
  ]
}
```

---

## Invariants and compatibility

- Every file has a top-level `version: 1`. Increment only on breaking schema changes.
- Writers use atomic writes (write to `*.tmp`, then rename) to survive crashes.
- Readers MUST tolerate unknown fields (forward compatibility).
- Readers MUST reject `version` values they do not recognize.
- Files are safe to commit only under an explicit user decision; by default
  they live in `whetstone/.state/` which SHOULD be gitignored.
