#!/bin/sh
# Minimal agent adapter: preserve stdout as JSON and branch on explicit state.
set -eu

response="$(wh check --json "$@" || true)"
state="$(printf '%s' "$response" | jq -er '.state')"

case "$state" in
  success) exit 0 ;;
  violated) printf '%s\n' "$response"; exit 1 ;;
  needs_decision|needs_input|stale|conflict|unknown|unavailable) printf '%s\n' "$response"; exit 2 ;;
  *) printf '%s\n' 'invalid Whetstone response' >&2; exit 3 ;;
esac
