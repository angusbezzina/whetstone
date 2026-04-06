# detect-patterns: Port to Rust or Keep as Python?

## Feature Summary

detect-patterns.py (~740 lines) mines 3 sources for recurring style signals:
1. Agent conversation transcripts (JSONL files)
2. Git commit history (via `git` CLI)
3. GitHub PR review comments (via `gh` CLI)

## Complexity Assessment

- Regex pattern matching against text
- JSONL file parsing
- subprocess calls to git and gh
- Grouping/deduplication/scoring heuristics
- ZERO external Python dependencies (stdlib only)

**Rust porting difficulty: LOW-MEDIUM**
- regex crate handles all pattern matching
- serde_json handles JSONL line-by-line
- std::process::Command handles subprocesses
- Estimated effort: 2-3 focused sessions

## Usage Frequency

- OPTIONAL in the doctor workflow
- Runs at most once per session
- NOT in the CI path
- Supplementary input to rule extraction, not core

## Recommendation: DEFER port, keep as optional companion

The code is simple enough to port, but it's low priority because:
1. Core Whetstone value is delivered without it
2. Pattern detection may evolve significantly
3. The script works standalone with zero dependencies
4. Porting adds no user-visible value until the "single binary" story matters for distribution

Current product stance:

- keep `scripts/detect-patterns.py` as an optional helper
- do not include it in the primary install-and-run workflow
- do not have `doctor` invoke it automatically

When the port happens, add it as `whetstone detect-patterns` only if it materially improves the main product story.
