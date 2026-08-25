#!/usr/bin/env bash
# Lift a lockout ban.
#
#   unban.sh <ip>      lift one address
#   unban.sh --list    show what is currently banned
#
# The ban lives in two places: a row in the key mirror, and the gateway's own
# memory. Deleting the row alone leaves the running process still refusing, so
# this restarts the gateway -- which reloads bans from the table it just changed.
set -euo pipefail
CONF_DIR="${QT_CONF_DIR:-/opt/qwen-turing/etc}"
. "${CONF_DIR}/common.conf"
DB="${QT_STATE}/keys.sqlite3"

[ $# -ge 1 ] || { sed -n '2,8p' "$0"; exit 64; }

if [ "$1" = "--list" ]; then
  sudo python3 - "$DB" <<'PY'
import sqlite3, sys, time
try:
    c = sqlite3.connect(sys.argv[1])
    rows = c.execute("SELECT ip, until, reason FROM lockout ORDER BY until").fetchall()
except sqlite3.Error as e:
    print(f"cannot read bans: {e}"); raise SystemExit(1)
now = time.time()
live = [r for r in rows if r[1] > now]
if not live:
    print("no active bans")
for ip, until, reason in live:
    print(f"{ip:<40} until {time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(until))}  ({reason})")
PY
  exit 0
fi

sudo python3 - "$DB" "$1" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
with c:
    n = c.execute("DELETE FROM lockout WHERE ip = ?", (sys.argv[2],)).rowcount
print(f"removed {n} ban row(s) for {sys.argv[2]}")
PY
sudo systemctl restart qwen-turing-gateway.service
echo "gateway restarted; the ban is lifted in memory too"
