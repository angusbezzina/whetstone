#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Repair or hydrate Whetstone's local Beads Dolt database.

Usage:
  scripts/beads-repair.sh [--role maintainer|contributor] [--remote <git-remote>] [--dry-run]

What it does:
  1. Stops the local Beads Dolt server if it is running
  2. Backs up any existing .beads/dolt directory under .beads/backup/local-db-repair/
  3. Ensures .beads/metadata.json points at the canonical Dolt database name: beads
  4. Clones the remote Dolt data into .beads/dolt/beads
  5. Optionally sets git config beads.role
  6. Restarts the Dolt server and prints verification commands

Examples:
  scripts/beads-repair.sh --role maintainer
  scripts/beads-repair.sh --role contributor --remote origin
  scripts/beads-repair.sh --dry-run
EOF
}

ROLE=""
REMOTE_NAME="origin"
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:-}"
      shift 2
      ;;
    --remote)
      REMOTE_NAME="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -n "$ROLE" && "$ROLE" != "maintainer" && "$ROLE" != "contributor" ]]; then
  echo "--role must be 'maintainer' or 'contributor'" >&2
  exit 1
fi

if ! command -v bd >/dev/null 2>&1; then
  echo "bd is required but not installed" >&2
  exit 1
fi

if ! command -v dolt >/dev/null 2>&1; then
  echo "dolt is required but not installed" >&2
  exit 1
fi

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

REMOTE_URL="$(git remote get-url "$REMOTE_NAME")"
if [[ -z "$REMOTE_URL" ]]; then
  echo "Could not determine git remote URL for '$REMOTE_NAME'" >&2
  exit 1
fi

TIMESTAMP="$(date +%Y%m%d%H%M%S)"
BACKUP_DIR=".beads/backup/local-db-repair/$TIMESTAMP"

run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '[dry-run] %s\n' "$*"
  else
    eval "$@"
  fi
}

echo "Using repo: $ROOT_DIR"
echo "Using git remote '$REMOTE_NAME': $REMOTE_URL"

run "bd dolt stop >/dev/null 2>&1 || true"
run "mkdir -p '$BACKUP_DIR' '.beads/dolt'"

if [[ -d .beads/dolt/beads || -d .beads/dolt/beads_whetstone ]]; then
  run "mv .beads/dolt '$BACKUP_DIR/dolt'"
  run "mkdir -p '.beads/dolt'"
fi

if [[ -f .beads/metadata.json ]]; then
  run "cp .beads/metadata.json '$BACKUP_DIR/metadata.json.bak'"
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  cat <<EOF
[dry-run] Would write .beads/metadata.json:
{
  "database": "dolt",
  "backend": "dolt",
  "dolt_database": "beads"
}
EOF
else
  cat > .beads/metadata.json <<'EOF'
{
  "database": "dolt",
  "backend": "dolt",
  "dolt_database": "beads"
}
EOF
fi

run "rm -rf '.beads/dolt/beads'"
run "cd '.beads/dolt' && dolt clone '$REMOTE_URL' beads"

if [[ -n "$ROLE" ]]; then
  run "git config beads.role '$ROLE'"
fi

run "bd dolt start"

cat <<'EOF'

Verification commands:
  bd context --json
  bd count
  bd ready
  bd dolt pull

If you still see Beads errors after repair, inspect:
  bd doctor
  bd dolt show
EOF
