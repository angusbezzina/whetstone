# Whetstone Troubleshooting

## Binary Not Found

**Symptom**: `whetstone: command not found`

**Fix**:
- If built from source: `cargo install --path .` or add `target/release/` to PATH
- If downloaded: ensure the binary is executable (`chmod +x whetstone`) and in PATH
- Verify: `which whetstone` or `whetstone --help`

## No Manifests Detected

**Symptom**: `whetstone detect-deps` returns 0 dependencies

**Fix**:
- Verify project has `pyproject.toml`, `package.json`, or `Cargo.toml`
- Check `--project-dir` points to the right directory
- Check `--exclude` isn't filtering out your manifests
- Note: `node_modules/`, `.venv/`, `target/`, and test fixtures are skipped by default

## Network Errors During resolve-sources

**Symptom**: Sources fail to resolve, timeout errors

**Fix**:
- Check internet connectivity
- Retry failed deps: `whetstone resolve-sources --retry-failed`
- Increase timeout: `--timeout 30`
- For rate-limited registries, reduce workers: `--workers 1`

## Stale Cache

**Symptom**: Whetstone returns outdated source content

**Fix**:
- Force refresh: `whetstone doctor --refresh` or `whetstone resolve-sources --force-refresh`
- Check cache age: `whetstone status --json` (look at `cache_stats`)
- Adjust TTL: `--ttl 86400` (1 day instead of default 7)

## No Rules After Doctor

**Symptom**: Doctor completes but no rules exist

**Cause**: Doctor detects dependencies and resolves docs, but rule extraction requires agent judgment. This is by design.

**Fix**:
1. Read the `extraction_context` from doctor output
2. Apply the Extraction Prompt for each source
3. Present proposed rules for approval
4. Save approved rules to `whetstone/rules/{language}/{dep}.yaml`

## State Corruption

**Symptom**: Unexpected errors referencing `.state/` files

**Fix**:
- Delete `whetstone/.state/` and re-run `whetstone doctor`
- State files are gitignored caches — safe to delete and regenerate

## CI Check Failing

**Symptom**: `whetstone ci-check` exits non-zero

**Cause**: Rules are stale or dependencies have version drift

**Fix**:
- Run `whetstone status` locally to see what drifted
- Run `whetstone doctor --changed-only` to update
- If intentional: adjust `--fail-on` threshold (e.g., `--fail-on none`)

## Inventory Shows Phantom Dependencies

**Symptom**: `pipeline_state` counts include deps no longer in manifests

**Fix**: Run `whetstone detect-deps --incremental` — this triggers stale-entry cleanup.
Dependencies with approved rules are protected from removal; others are cleaned up automatically.
