# Performance Baselines

These baselines capture expected order-of-magnitude behavior for the Rust
binary on representative repositories. They are intended for regression review,
not strict microbenchmark gating.

## Representative Repositories

- **Whetstone** — self-host sanity check
- **Splinter** — mixed-language monorepo benchmark target

## Current Baseline Ranges

### Splinter

| Scenario | Expected Range | Notes |
|----------|----------------|-------|
| Cold `whetstone doctor` fast-first default | ~5-10s | ranked subset first |
| Follow-up `whetstone doctor --resume` | ~15-30s | completes remaining resolution |
| Warm `whetstone doctor --changed-only` | sub-second to ~2s | mostly cache-bound |
| `whetstone status --json` | sub-second to ~2s | local state only |

### Whetstone

| Scenario | Expected Range | Notes |
|----------|----------------|-------|
| `whetstone detect-deps` | sub-second to low seconds | local manifests only |
| `whetstone status --json` | sub-second to low seconds | local state/rules only |

## Re-measuring

```bash
/usr/bin/time -p whetstone doctor --project-dir /path/to/repo --skip-patterns
/usr/bin/time -p whetstone doctor --project-dir /path/to/repo --skip-patterns --resume
/usr/bin/time -p whetstone doctor --project-dir /path/to/repo --skip-patterns --changed-only
/usr/bin/time -p whetstone status --project-dir /path/to/repo --json --no-drift-check
```

## Interpretation

- Judge regressions against the scenario class, not exact second-for-second output.
- The most important product guarantees are fast first value and strong warm-cache performance.
- If a change materially worsens behavior outside these bands, capture it in the relevant migration or performance bead before closing the work.
