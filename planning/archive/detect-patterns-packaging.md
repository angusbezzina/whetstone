# detect-patterns Packaging Story

## Current Decision

`detect-patterns` remains an **optional standalone Python helper** and is **not** part of the Rust binary command surface.

## Supported Product Boundary

- Core product: Rust binary (`whetstone ...`)
- Optional helper: `scripts/detect-patterns.py`
- Doctor workflow: does **not** invoke pattern mining automatically

## Packaging Stance

- Do **not** include detect-patterns in the primary install path or simple binary workflow
- Document it only as an optional advanced helper for transcript/git/PR mining
- Do not require Python for the core product story

## Why

- Core deterministic workflows deliver the main product value without it
- Pattern mining depends on environment-specific transcript, git, and gh state
- It is supplementary rather than required for install-and-run adoption

## Future Revisit Criteria

Only reconsider a Rust port if:

1. users explicitly ask for pattern mining as part of the main product
2. the single-binary install story remains clear
3. privacy and environment assumptions are acceptable
