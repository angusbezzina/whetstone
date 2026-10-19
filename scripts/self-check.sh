#!/usr/bin/env bash
# Whetstone checks itself: the required CI check (`wh check --ci`) over the
# whole tree, against the must rules accepted and shared in Whetstone's own
# Beads store. Fails on any must finding, on evidence that is not a pass, and
# when no must rule ran.
#
# Usage: scripts/self-check.sh [--bootstrap] <whetstone binary>
#   --bootstrap  for a fresh CI checkout: fetch the Beads data
#                (refs/dolt/data) from origin with the checkout's own
#                credentials, then clone the shared database from that local
#                copy. The tracked sync remote is SSH, which CI cannot reach.
set -euo pipefail

bootstrap=false
if [ "${1:-}" = --bootstrap ]; then
  bootstrap=true
  shift
fi
bin=$(cd "$(dirname "${1:?usage: scripts/self-check.sh [--bootstrap] <whetstone binary>}")" && pwd)/$(basename "$1")
cd "$(git rev-parse --show-toplevel)"

if $bootstrap; then
  git fetch --quiet --no-tags origin +refs/dolt/data:refs/dolt/data
  BD_NON_INTERACTIVE=1 BD_SYNC_REMOTE="git+file://$PWD/.git" bd bootstrap --yes >/dev/null
  # Bootstrap writes the local remote into the tracked config; the check only reads.
  git checkout --quiet -- .beads
fi

result=$("$bin" check --ci --json) || true
echo "$result" | jq -r '.data.gates[]? | "  \(.state) \(.strength) \(.id): \(.summary)"'
if ! echo "$result" | jq -e '
  .state == "success" and
  ([.data.gates[]? | select(.strength == "must")] | length) > 0
' >/dev/null; then
  echo "$result" | jq -r '.summary' >&2
  exit 1
fi
