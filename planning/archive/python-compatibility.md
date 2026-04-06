# Python Compatibility Boundaries

## Classification

| Script | Status | Rationale |
|--------|--------|-----------|
| `scripts/legacy/detect-deps.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/resolve-sources.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/doctor.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/status.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/ci-check.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/generate-tests.py` | Archived/reference | Full Rust parity |
| `scripts/legacy/generate-agent-context.py` | Archived/reference | Full Rust parity |
| `scripts/detect-patterns.py` | ACTIVE | Only unported feature |
| `scripts/legacy/state.py` | Support | Used by archived Python scripts |
| `scripts/legacy/cli.py` | Archived | Superseded Python CLI wrapper |

## What "Python-Free" Means

1. Installing a release binary or running `cargo install` is sufficient for ALL core workflows
2. No Python runtime needed for: doctor, detect-deps, resolve-sources, status, ci-check, generate-context, generate-tests
3. Pattern detection (detect-patterns) is the ONLY feature requiring Python
4. Pattern detection is OPTIONAL -- doctor workflow skips it by default
5. A user who never installs Python gets full value from Whetstone

## Retention Policy

- archived command scripts live under `scripts/legacy/`
- No Python script is invoked by the Rust binary
- Python tests remain for cross-validation during transition
- archived scripts should NOT be included in release artifacts or install instructions

## Development vs User Python

The quality gates in CLAUDE.md (`ruff check`, `pytest`) are for Whetstone's own development. They are NOT user-facing requirements.
