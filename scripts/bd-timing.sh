#!/usr/bin/env bash
# Logs every bd call's wall seconds and subcommand, for finding where test
# time goes. Usage: scripts/bd-timing.sh <log> <command...>, for example
#   scripts/bd-timing.sh /tmp/bd.log cargo test --test push_gate
# then: awk '{t[$2]+=$1; n[$2]++} END {for (k in t) print k, n[k], t[k]}' /tmp/bd.log
set -euo pipefail
log=$(cd "$(dirname "$1")" && pwd)/$(basename "$1"); shift
real=$(command -v bd)
shim=$(mktemp -d)
cat > "$shim/bd" <<SHIM
#!/usr/bin/env bash
start=\$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
"$real" "\$@"; status=\$?
perl -MTime::HiRes=time -e 'printf "%.3f %s\n", time - \$ARGV[0], \$ARGV[1]' "\$start" "\${1:-}" >> "$log"
exit \$status
SHIM
chmod +x "$shim/bd"
PATH="$shim:$PATH" "$@"
