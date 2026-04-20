#!/usr/bin/env bash
# Epic 3E measurement harness.
#
# Captures token cost, time-to-gauge-repo-health, and runtime
# for a given project directory, against the current whetstone
# binary. Re-run after each theme lands to compare against the
# baseline in planning/measurements/epic-3e-baseline.md.
#
# Usage:
#   scripts/measure-epic-3e.sh <project-dir> [label]
#
# Writes a one-line markdown row to stdout plus full metrics to stderr.

set -euo pipefail

PROJECT_DIR="${1:-tests/fixtures}"
LABEL="${2:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
BIN="${WHETSTONE_BIN:-./target/release/whetstone}"

if [[ ! -x "$BIN" ]]; then
    echo "Binary not found at $BIN. Run: cargo build --release" >&2
    exit 1
fi

if [[ ! -d "$PROJECT_DIR" ]]; then
    echo "Project directory not found: $PROJECT_DIR" >&2
    exit 1
fi

echo "=== Epic 3E measurement: $LABEL ===" >&2
echo "project_dir: $PROJECT_DIR" >&2
echo "binary: $BIN" >&2
echo "" >&2

# Regenerate outputs so we measure the current state, not a stale file.
"$BIN" actions --project-dir "$PROJECT_DIR" > /dev/null 2>&1 || true

AGENTS_MD="$PROJECT_DIR/whetstone/context/AGENTS.md"
if [[ -f "$AGENTS_MD" ]]; then
    AGENTS_BYTES=$(wc -c < "$AGENTS_MD" | tr -d ' ')
    AGENTS_LINES=$(wc -l < "$AGENTS_MD" | tr -d ' ')
    AGENTS_TOKENS=$(( AGENTS_BYTES / 4 ))
else
    AGENTS_BYTES=0
    AGENTS_LINES=0
    AGENTS_TOKENS=0
fi

# Time wh status (average of 3 runs, seconds, 3 decimal places).
time_cmd() {
    local total=0
    for _ in 1 2 3; do
        local start end
        start=$(date +%s%N)
        "$@" > /dev/null 2>&1 || true
        end=$(date +%s%N)
        total=$((total + end - start))
    done
    # Average in milliseconds.
    awk "BEGIN { printf \"%.3f\", ($total / 3) / 1000000 }"
}

STATUS_MS=$(time_cmd "$BIN" status --json --no-snapshot --no-drift-check --project-dir "$PROJECT_DIR")
RULES_QUERY_MS=$(time_cmd "$BIN" rules query --file "$PROJECT_DIR/src/dummy.py" --json --project-dir "$PROJECT_DIR" 2>/dev/null || echo "n/a")

# Human-readable metrics to stderr.
{
    echo "AGENTS.md bytes: $AGENTS_BYTES"
    echo "AGENTS.md lines: $AGENTS_LINES"
    echo "AGENTS.md ~tokens (bytes/4): $AGENTS_TOKENS"
    echo "wh status avg runtime (3 runs): ${STATUS_MS}ms"
    echo "wh rules query avg runtime (3 runs): ${RULES_QUERY_MS}ms"
} >&2

# Machine-readable markdown row to stdout for appending to the measurement log.
printf "| %s | %s | %s | %s | %s | %s |\n" \
    "$LABEL" \
    "$PROJECT_DIR" \
    "$AGENTS_BYTES" \
    "$AGENTS_TOKENS" \
    "${STATUS_MS}ms" \
    "${RULES_QUERY_MS}ms"
