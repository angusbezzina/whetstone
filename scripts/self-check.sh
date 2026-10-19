#!/usr/bin/env bash
# Whetstone checks itself: the required CI check (`wh check --ci`) over the
# whole tree, against the must rules in Whetstone's own committed rulebook
# (whetstone/rulebook.json). Fails on any must finding, on evidence that is
# not a pass, and when no must rule ran.
#
# Usage: scripts/self-check.sh <whetstone binary>
set -euo pipefail

bin=$(cd "$(dirname "${1:?usage: scripts/self-check.sh <whetstone binary>}")" && pwd)/$(basename "$1")
cd "$(git rev-parse --show-toplevel)"

result=$("$bin" check --ci --json) || true
echo "$result" | jq -r '.data.gates[]? | "  \(.state) \(.strength) \(.id): \(.summary)"'
if ! echo "$result" | jq -e '
  .state == "success" and
  ([.data.gates[]? | select(.strength == "must")] | length) > 0
' >/dev/null; then
  echo "$result" | jq -r '.summary' >&2
  exit 1
fi
