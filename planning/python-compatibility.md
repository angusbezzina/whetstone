# Python Compatibility Boundaries

## Classification

| Script | Status | Rationale |
|--------|--------|-----------|
| detect-deps.py | Legacy/reference | Full Rust parity |
| resolve-sources.py | Legacy/reference | Full Rust parity |
| doctor.py | Legacy/reference | Full Rust parity |
| status.py | Legacy/reference | Full Rust parity |
| ci-check.py | Legacy/reference | Full Rust parity |
| generate-tests.py | Legacy/reference | Full Rust parity |
| generate-agent-context.py | Legacy/reference | Full Rust parity |
| detect-patterns.py | ACTIVE | Only unported feature |
| state.py | Support | Used by other Python scripts |
| cli.py | Dormant | Python CLI wrapper, superseded by Rust binary |

## What "Python-Free" Means

1. Installing a release binary or running `cargo install` is sufficient for ALL core workflows
2. No Python runtime needed for: doctor, detect-deps, resolve-sources, status, ci-check, generate-context, generate-tests
3. Pattern detection (detect-patterns) is the ONLY feature requiring Python
4. Pattern detection is OPTIONAL -- doctor workflow skips it by default
5. A user who never installs Python gets full value from Whetstone

## Retention Policy

- scripts/ directory stays in repo as reference implementation
- No Python script is invoked by the Rust binary
- Python tests remain for cross-validation during transition
- scripts/ should NOT be included in release artifacts or install instructions

## Development vs User Python

The quality gates in CLAUDE.md (`ruff check`, `pytest`) are for Whetstone's own development. They are NOT user-facing requirements.
