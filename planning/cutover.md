# Python to Rust Cutover Criteria

## Per-Command Status

| Command | Rust Parity | Python Status | Cutover Ready |
|---------|------------|---------------|---------------|
| `detect-deps` | Full | Legacy | Yes |
| `resolve-sources` | Full | Legacy | Yes |
| `doctor` | Full (3-step, no pattern detection) | Legacy | Yes |
| `status` | Full | Legacy | Yes |
| `ci-check` | Full | Legacy | Yes |
| `generate-context` | Full | Legacy | Yes |
| `generate-tests` | Full | Legacy | Yes |
| `detect-patterns` | Not ported (deferred) | Active | No — Python script remains primary for pattern mining |

## What "Legacy" Means

Python reference scripts in `scripts/legacy/` are retained for:
1. Reference implementation during validation
2. Pattern detection (only feature not in Rust)
3. Backwards compatibility during transition

They are NOT the recommended path for any command except `detect-patterns`.

## Cutover Criteria Per Command

A command is ready for Rust-only when:
1. Rust output matches Python output for all tested scenarios
2. Rust has integration test coverage
3. Docs reference the Rust binary as primary
4. CI validates the Rust path

## Remaining Python Dependency

The only Python-exclusive functionality is `scripts/detect-patterns.py` which mines agent conversation transcripts, git history, and PR comments for style patterns. This is deferred from the Rust port because:
- It requires shell-out to `git` and `gh` CLI tools
- Its value is supplementary (patterns are optional input to extraction)
- The feature is lower priority than core workflow correctness

## Next Steps

1. Remove Python script references from user-facing docs (done)
2. Add `detect-patterns` to Rust roadmap when pattern mining demand warrants it
3. Consider removing Python scripts entirely once pattern detection is ported
