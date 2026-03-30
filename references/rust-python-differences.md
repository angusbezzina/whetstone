# Rust vs Python Implementation Differences

This document records intentional behavioral differences between the Rust binary (`src/`) and the Python reference scripts (`scripts/`).

## Intentional Differences

### detect-patterns not ported

The Python `scripts/detect-patterns.py` mines agent transcripts, git history, and PR comments for recurring style patterns. This is not ported to Rust. The doctor workflow always reports `patterns_found: 0` and `extraction_context.patterns: []`.

### Dependency ordering

Python sorts dependencies alphabetically by name within each language group. Rust uses `BTreeMap<(String, String, bool)>` keyed by `(language, name, dev)`, which produces alphabetical order but groups by language first.

### Timestamp precision

Python uses `datetime.utcnow().isoformat()` (microsecond precision, no timezone). Rust uses `chrono::Utc::now().to_rfc3339()` (nanosecond precision with `+00:00` timezone offset).

### Version string normalization

Python `detect-deps.py` strips version constraint prefixes (`>=`, `^`, `~`) to produce bare version numbers. Rust preserves the full version constraint string as written in the manifest.

### count_project_deps in status

Python `status.py` reads the inventory for total dependency counts. Rust `status.rs` calls `detect_deps()` fresh (via `count_project_deps()`) to always reflect current manifests. As of crd.1.1, Rust also reads `detected_totals` from the inventory when available, reconciling with the last `detect-deps` run.

### Stale entry cleanup

Rust (as of crd.1.2) removes inventory entries for dependencies no longer present in manifests, unless they have approved rules. Python does not perform this cleanup — stale entries persist indefinitely.

## Fields present in Python but not Rust

| Field | Reason |
|-------|--------|
| `extraction_context.patterns` | Always `[]` in Rust (detect-patterns not ported) |
| `patterns_found` | Always `0` in Rust |

## Fields present in Rust but not Python

| Field | Purpose |
|-------|---------|
| `discovery.excluded` | Full effective exclusion list |
| `discovery.included` | Explicit include overrides |
| `scan.ranked_queue` | Doctor dependency priority ranking |
| `pipeline_state.inventory_entries` | Raw inventory entry count (distinct from reconciled total_deps) |
| `inventory_diff.actually_removed` | Keys cleaned up by stale-entry removal |
| `inventory_diff.protected` | Keys protected from removal (have approved rules) |
