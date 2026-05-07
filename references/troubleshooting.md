# Whetstone Troubleshooting

## Binary not found

**Symptom:** `whetstone: command not found`

**Fix**
- If built from source: `cargo install --path .`
- If downloaded: ensure the binary is executable and on your `PATH`
- Verify: `which whetstone` or `whetstone --help`

## No manifests detected

**Symptom:** `wh init --detect-only` returns 0 dependencies

**Fix**
- Verify the project has `pyproject.toml`, `package.json`, or `Cargo.toml`
- Check `--project-dir` points to the correct repo
- Check discovery include/exclude settings in `whetstone/whetstone.yaml`

## Network or source resolution errors

**Symptom:** `wh init` or `wh reinit` cannot resolve documentation/sources

**Fix**
- Check internet connectivity
- Retry with a larger timeout via config (`resolve.timeout_seconds`) or CLI flag
- For weak custom sources, inspect them with:

```bash
wh sources list
wh sources verify <name-or-url>
```

- For stale dependency sources, resume the bootstrap flow:

```bash
wh init --resume
```

## Stale cache or stale source content

**Symptom:** Whetstone appears to use outdated source material

**Fix**
- Run `wh reinit`
- Force a custom source refresh with `wh sources verify <name-or-url>`
- Inspect cache-related health via `wh status --json`

## No rules after init

**Symptom:** Bootstrap succeeds, but no rules appear

**Cause:** Whetstone prepares source material and worklists; extraction and
approval still require agent/user judgment.

**Fix**
```bash
wh sources list
wh rules worklist
wh extract
wh extract submit <bundle.yaml>
wh rules approve --all --confidence high
wh actions all
wh scan src/
```

## Config validation fails

**Symptom:** `wh config validate` exits non-zero

**Fix**
- Run `wh config show` to inspect the effective config stack
- Check imported pack refs under `extends:`
- Prefer canonical project config at `whetstone/whetstone.yaml`
- Keep repo-root `whetstone.yaml` only as a temporary compatibility fallback

## Unexpected `.state/` issues

**Symptom:** Errors mention files under `whetstone/.state/`

**Fix**
- These are cache/state artifacts and are generally safe to regenerate
- Remove `whetstone/.state/` and rerun `wh init` or `wh reinit`

## CI check failing

**Symptom:** `wh ci` exits non-zero

**Fix**
- Run `wh status` to inspect freshness/adherence
- Run `wh reinit` if drift exists
- Run `wh scan` to inspect violations
- Adjust `--fail-on` only if the failure policy itself is incorrect

## Inventory shows stale dependencies

**Symptom:** Status still mentions dependencies no longer present in manifests

**Fix**
- Run `wh init --detect-only --incremental`
- Then rerun `wh init` or `wh reinit` to refresh the stored state
