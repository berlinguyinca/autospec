#!/usr/bin/env bash
# autospec-rollover-status/show.sh — read-only context monitor status reporter
# Reports current context % and last rollover event from the active monitor log.
set -eu

dir="${HOME}/.autospec/monitors"

if [[ ! -d "$dir" ]]; then
    echo "No monitor logs at $dir (no session active)"
    exit 0
fi

log=$(ls -t "$dir"/*.log 2>/dev/null | head -1) || true

if [[ -z "${log:-}" ]]; then
    echo "No monitor sessions active (no *.log files in $dir)"
    exit 0
fi

echo "Monitor log: $log"
echo "---"

# Show last 5 usage/rollover/compact/handoff events
recent=$(tail -100 "$log" | grep -E '"event":"(usage|rollover|compact|handoff|clear|resume|noop)"' | tail -5)

if [[ -z "$recent" ]]; then
    echo "(no usage or rollover events recorded yet)"
else
    echo "$recent" | while IFS= read -r line; do
        # Extract fields for human-readable display
        event=$(printf '%s' "$line" | grep -o '"event":"[^"]*"' | sed 's/"event":"//;s/"//')
        ts=$(printf '%s' "$line" | grep -o '"ts":"[^"]*"' | sed 's/"ts":"//;s/"//')
        pct=$(printf '%s' "$line" | grep -o '"pct":[0-9.]*' | sed 's/"pct"://')
        used=$(printf '%s' "$line" | grep -o '"used":[0-9]*' | sed 's/"used"://')
        max=$(printf '%s' "$line" | grep -o '"max":[0-9]*' | sed 's/"max"://')

        if [[ -n "$pct" ]]; then
            pct_display=$(awk "BEGIN{printf \"%.1f%%\", $pct * 100}" 2>/dev/null || echo "${pct}")
            echo "  [${ts:-?}] ${event}: ${pct_display} (${used:-?}/${max:-?} tokens)"
        else
            echo "  [${ts:-?}] ${event}"
        fi
    done
fi

# Show most recent context percentage summary
last_pct=$(tail -200 "$log" | grep '"event":"usage"' | tail -1 | grep -o '"pct":[0-9.]*' | sed 's/"pct"://')
if [[ -n "$last_pct" ]]; then
    pct_human=$(awk "BEGIN{printf \"%.1f\", $last_pct * 100}" 2>/dev/null || echo "$last_pct")
    echo "---"
    echo "Current context: ${pct_human}% used"
fi
