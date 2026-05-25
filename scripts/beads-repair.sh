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
  4. Writes .beads/dolt-server.port so bd never falls back to port 0
  5. Clones the remote Dolt data into .beads/dolt/beads
  6. Optionally sets git config beads.role
  7. Restarts the Dolt server and prints verification commands

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

choose_port() {
  is_port_available() {
    python3 - "$1" <<'PY'
import socket
import sys

port = int(sys.argv[1])
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.settimeout(0.05)
    raise SystemExit(0 if sock.connect_ex(("127.0.0.1", port)) != 0 else 1)
PY
  }

  local configured
  configured="$(python3 - <<'PY'
from pathlib import Path
import re

config = Path(".beads/config.yaml")
if config.exists():
    match = re.search(r"(?m)^dolt\.port:\s*([0-9]+)\s*$", config.read_text())
    if match and match.group(1) != "0":
        print(match.group(1))
PY
)"
  if [[ -n "$configured" ]] && is_port_available "$configured"; then
    printf '%s\n' "$configured"
    return
  fi

  if [[ -f .beads/dolt-server.port ]]; then
    local existing
    existing="$(tr -cd '0-9' < .beads/dolt-server.port)"
    if [[ -n "$existing" && "$existing" != "0" ]] && is_port_available "$existing"; then
      printf '%s\n' "$existing"
      return
    fi
  fi

  python3 - "$ROOT_DIR" <<'PY'
import hashlib
import socket
import sys

root = sys.argv[1].encode()
start = 61000 + (int(hashlib.sha256(root).hexdigest()[:8], 16) % 3000)
for offset in range(3000):
    port = 61000 + ((start - 61000 + offset) % 3000)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.05)
        if sock.connect_ex(("127.0.0.1", port)) != 0:
            print(port)
            raise SystemExit(0)
raise SystemExit("could not find an available localhost port in 61000-63999")
PY
}

write_config_port() {
  local port="$1"
  python3 - "$port" <<'PY'
from pathlib import Path
import re
import sys

port = sys.argv[1]
path = Path(".beads/config.yaml")
body = path.read_text() if path.exists() else ""
line = f"dolt.port: {port}"
if re.search(r"(?m)^dolt\.port:\s*[0-9]+\s*$", body):
    body = re.sub(r"(?m)^dolt\.port:\s*[0-9]+\s*$", line, body)
else:
    body = body.rstrip() + "\n\n# Beads Dolt SQL server port. Keep explicit so bd never falls back to port 0.\n" + line + "\n"
path.write_text(body)
PY
}

echo "Using repo: $ROOT_DIR"
echo "Using git remote '$REMOTE_NAME': $REMOTE_URL"

run "bd dolt stop >/dev/null 2>&1 || true"
run "bd dolt killall >/dev/null 2>&1 || true"

PORT="$(choose_port)"
echo "Using Dolt SQL port: $PORT"

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

if [[ "$DRY_RUN" -eq 1 ]]; then
  printf '[dry-run] Would write .beads/dolt-server.port: %s\n' "$PORT"
  printf '[dry-run] Would ensure .beads/config.yaml contains: dolt.port: %s\n' "$PORT"
else
  printf '%s\n' "$PORT" > .beads/dolt-server.port
  write_config_port "$PORT"
fi

run "rm -f '.beads/dolt-server.lock' '.beads/dolt-server.pid' '.beads/dolt/.dolt/sql-server.info'"
run "rm -rf '.beads/dolt/beads'"
run "cd '.beads/dolt' && dolt clone '$REMOTE_URL' beads"

if [[ -n "$ROLE" ]]; then
  run "git config beads.role '$ROLE'"
fi

run "bd dolt start"
run "bd dolt test"

cat <<'EOF'

Verification commands:
  bd dolt show
  bd context --json
  bd count
  bd ready
  bd dolt pull

If you still see Beads errors after repair, inspect:
  bd doctor
  bd dolt show
EOF
