#!/usr/bin/env bash
# growth-measure-due.sh — Tier G3 cadence gate for run-growth-measure.
# Usage: growth-measure-due.sh <repo_root>
# Prints "1" if now - <last "measure" ledger line ts> >= grow.measure_interval
# (config .autospec/growth.yml, default 14 days), else "0". If the ledger has
# no prior "measure" line at all (never measured), prints "1" — a fresh repo
# should still get its first measure cycle.
#
# Fail-closed to "0" on any missing/unreadable ledger, missing jq, or a
# malformed ledger — a broken environment must never spuriously trigger a
# growth-measure cycle.
set -euo pipefail

DAY_SECONDS=86400
DEFAULT_INTERVAL_DAYS=14

now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }

not_due() { printf '0\n'; exit 0; }

repo_root="${1:-.}"

command -v jq >/dev/null 2>&1 || not_due

ledger="${GROWTH_LEDGER:-${repo_root}/.autospec/growth/ledger.jsonl}"
[ -f "$ledger" ] || not_due
[ -r "$ledger" ] || not_due

# Malformed-ledger guard (mirrors growth-ethics-precheck.sh's check_cadence).
if ! jq . "$ledger" >/dev/null 2>&1; then
    not_due
fi

# Resolve the configured measure interval (days); default 14. Best-effort —
# a missing/invalid config or missing yq falls back to the default.
interval_days="$DEFAULT_INTERVAL_DAYS"
config="${repo_root}/.autospec/growth.yml"
if [ -f "$config" ] && command -v yq >/dev/null 2>&1; then
    configured=""
    configured="$( \
        (yq -o=json '.' "$config" 2>/dev/null || yq '.' "$config" 2>/dev/null) \
            | jq -r '.grow.measure_interval // empty' 2>/dev/null \
    )" || configured=""
    case "$configured" in
        ''|*[!0-9]*) : ;;
        *) interval_days="$configured" ;;
    esac
fi
interval_seconds=$(( interval_days * DAY_SECONDS ))

now="$(now_epoch)"

# Count measure lines independently of ts-parseability so we can tell
# "never measured" (genuinely due) apart from "measure lines exist but every
# ts is malformed" (a broken ledger — must fail-closed, not spuriously fire).
measure_count=""
measure_count="$(jq -s -r '[.[] | select(.kind == "measure")] | length' "$ledger" 2>/dev/null)" || measure_count=""
case "$measure_count" in ''|*[!0-9]*) not_due ;; esac

last_ts_epoch=""
last_ts_epoch="$(jq -s -r '
    map(select(.kind == "measure"))
    | map(.ts | (if type=="number" then . else (fromdateiso8601? // empty) end))
    | map(select(. != null))
    | sort
    | last // empty
' "$ledger" 2>/dev/null)" || last_ts_epoch=""

if [ -z "$last_ts_epoch" ]; then
    if [ "$measure_count" -gt 0 ]; then
        # Measure lines exist but none carries a parseable ts — malformed
        # ledger. Fail-closed to not-due rather than treating it as "never
        # measured" and firing a spurious measure cycle.
        not_due
    fi
    # No measure line at all — never measured, so it is due.
    printf '1\n'
    exit 0
fi

case "$last_ts_epoch" in
    *[!0-9.]*|'') not_due ;;
esac
last_ts_epoch="${last_ts_epoch%%.*}"

age=$(( now - last_ts_epoch ))
if [ "$age" -ge "$interval_seconds" ]; then
    printf '1\n'
else
    printf '0\n'
fi
