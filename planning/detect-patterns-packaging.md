# detect-patterns Packaging Story

## Context

detect-patterns remains a Python script while the rest of Whetstone is a Rust binary. This document defines how it's distributed and discovered.

## Recommended Approach: Bundled Script

Ship detect-patterns.py alongside the binary in release artifacts:

1. Include `scripts/detect-patterns.py` in GitHub Release assets
2. Binary detects Python availability via `which python3`
3. Future: add `whetstone detect-patterns` subcommand that shells out to the script
4. If Python not available: print helpful error, exit with specific code

### Why this works
- The script has ZERO external Python dependencies (stdlib only)
- Any system with Python 3.9+ can run it without pip install
- Users who want pattern detection already likely have Python installed
- No PyPI publishing overhead

## Integration Points

- cli.rs: Future `DetectPatterns` variant in Commands enum
- doctor.rs: Wire `skip_patterns` flag to optionally invoke detect-patterns
- Output contract: detect-patterns.py already emits JSON matching project conventions

## Alternative Considered: pip install

A `whetstone-patterns` PyPI package was considered but rejected because:
- Extra install step for users
- Publishing overhead
- The script is a single file with no dependencies
