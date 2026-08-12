#!/usr/bin/env bash
# scripts/lib/opencode-usage-probe.sh — OpenCode live-usage observability probe.
#
# Wires the usage-observe.sh probe seam (AUTOSPEC_USAGE_PROBE_OPENCODE) to a real
# signal: OpenCode persists per-session token usage (tokens_input / tokens_output
# / tokens_reasoning) in its SQLite DB at $OPENCODE_DB_PATH (default
# ~/.local/share/opencode/opencode.db). This probe sums the billed token classes
# for sessions active within the trailing window and prints that total as a
# percentage of AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS.
#
# Emits a single number 0-100 (one decimal) on stdout, or exits non-zero when the
# DB is unreadable or the ceiling is unusable — usage-observe.sh then reports the
# honest observable=false fallback rather than a fabricated fraction.
#
# This is a trailing-window cumulative tally, not a provider quota fraction: the
# provider resets its quota on its own schedule and OpenCode does not surface it.
# It is still a strictly better signal than observable=false, which gives the
# conductor no OpenCode-specific usage information at all.
#
# Env:
#   OPENCODE_DB_PATH                     default ~/.local/share/opencode/opencode.db
#   OPENCODE_USAGE_WINDOW_HOURS          trailing window (default 24 = daily proxy)
#   AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  denominator ceiling (default 50000000)

set -eu

db="${OPENCODE_DB_PATH:-$HOME/.local/share/opencode/opencode.db}"
window_hours="${OPENCODE_USAGE_WINDOW_HOURS:-24}"
ceiling="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-50000000}"

[ -f "$db" ] || exit 1
command -v python3 >/dev/null 2>&1 || exit 1

used="$(python3 - "$db" "$window_hours" <<'PY'
import sqlite3, sys, time
db, window_hours = sys.argv[1], float(sys.argv[2])
conn = sqlite3.connect(db)
cutoff_ms = int((time.time() - window_hours * 3600) * 1000)
row = conn.execute(
    "SELECT COALESCE(SUM(tokens_input),0), COALESCE(SUM(tokens_output),0),"
    " COALESCE(SUM(tokens_reasoning),0) FROM session WHERE time_updated >= ?",
    (cutoff_ms,),
).fetchone()
print(sum(row) if row else 0)
PY
)"

case "$used" in
    ''|*[!0-9]*) exit 1 ;;
esac

awk -v u="$used" -v c="$ceiling" 'BEGIN {
    if (c <= 0) exit 1
    p = u / c * 100
    if (p > 100) p = 100
    printf "%.1f\n", p
}'
