#!/usr/bin/env bash
# autospec-watchdog.sh — reclaim and nudge stalled autospec workers.
#
# The monitor should call this every 12 iterations (or on the same cadence) to
# detect stalled `process-heartbeats/*.json` files and:
# 1) leave a reminder comment
# 2) reclaim the issue (if stalled beyond reclaim threshold)
#
# Environment overrides:
#   AUTOSPEC_WATCHDOG_DIR              heartbeat directory (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_WATCHDOG_REPO              override repo for gh calls (default: gh repo context)
#   AUTOSPEC_WATCHDOG_STALE_SECS         stale threshold (default: 1800)
#   AUTOSPEC_WATCHDOG_RECLAIM_SECS       reclaim threshold (default: 10800)
#   AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS nudge cooldown (default: 900)
#   AUTOSPEC_WATCHDOG_STATE_FILE         state file for nudge cooldown (default: ~/.autospec/watchdog-state.tsv)

set -eu

WATCHDOG_DIR="${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}"
WATCHDOG_REPO="${AUTOSPEC_WATCHDOG_REPO:-${AUTOSPEC_REPO:-}}"
WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
WATCHDOG_NUDGE_COOLDOWN_SECS="${AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS:-900}"
STATE_FILE="${AUTOSPEC_WATCHDOG_STATE_FILE:-$HOME/.autospec/watchdog-state.tsv}"

WATCHDOG_LOG_PREFIX="[autospec-watchdog]"

if ! command -v gh >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX ERROR: gh CLI not found" >&2
    exit 1
fi

if [ ! -d "$WATCHDOG_DIR" ]; then
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 skipped=0"
    exit 0
fi

now_ts="$(date -u +%s)"
nudged=0
reclaimed=0
skipped=0

if [ -n "$WATCHDOG_REPO" ]; then
    repo_args=(--repo "$WATCHDOG_REPO")
else
    repo_args=()
fi

declare -A LAST_NUDGE_TS

load_state() {
    if [ -f "$STATE_FILE" ]; then
        while IFS=$'\t' read -r issue ts; do
            if [[ "$issue" =~ ^[0-9]+$ ]] && [[ "$ts" =~ ^[0-9]+$ ]]; then
                LAST_NUDGE_TS["$issue"]="$ts"
            fi
        done < "$STATE_FILE"
    fi
}

extract_ts() {
    awk -F: '
        /"ts"[[:space:]]*:/ {
            line=$0
            gsub(/^[[:space:]]*/, "", line)
            if (match(line, /"ts"[[:space:]]*:[[:space:]]*([0-9]+)/, m)) {
                print m[1]
                exit
            }
        }
    ' "$1"
}

issue_meta() {
    gh issue view "$1" "${repo_args[@]}" \
        --json state,labels \
        --jq '.state + " " + ([.labels[].name] | join(","))' \
        2>/dev/null || true
}

reclaim_issue() {
    local issue="$1"
    local age="$2"

    gh issue edit "$issue" "${repo_args[@]}" \
        --remove-label in-progress-by-bot \
        --add-label auto-implement >/dev/null 2>&1 || true
    gh issue comment "$issue" "${repo_args[@]}" \
        --body "autospec watchdog reclaimed this issue after ${age}s of no check-in." >/dev/null 2>&1 || true
}

nudge_issue() {
    local issue="$1"

    gh issue comment "$issue" "${repo_args[@]}" \
        --body "autospec watchdog: please check in; if stuck, post blocker and clear in-progress-by-bot." \
        >/dev/null 2>&1 || return 1
}

save_state() {
    mkdir -p "$HOME/.autospec"
    if [ "${#LAST_NUDGE_TS[@]}" -eq 0 ]; then
        rm -f "$STATE_FILE"
        return
    fi
    tmp="$(mktemp "$HOME/.autospec/.watchdog-state.XXXXXX")"
    for issue in "${!LAST_NUDGE_TS[@]}"; do
        printf '%s\t%s\n' "$issue" "${LAST_NUDGE_TS[$issue]}" >> "$tmp"
    done
    mv "$tmp" "$STATE_FILE"
}

load_state

for hb in "$WATCHDOG_DIR"/*.json; do
    [ -f "$hb" ] || continue

    issue="${hb##*/}"
    issue="${issue%.json}"
    if [[ ! "$issue" =~ ^[0-9]+$ ]]; then
        skipped=$((skipped + 1))
        continue
    fi

    ts="$(extract_ts "$hb")"
    if [[ ! "$ts" =~ ^[0-9]+$ ]]; then
        skipped=$((skipped + 1))
        continue
    fi

    age=$(( now_ts - ts ))
    if [ "$age" -lt 0 ]; then
        age=0
    fi
    if [ "$age" -lt "$WATCHDOG_STALE_SECS" ]; then
        continue
    fi

    meta="$(issue_meta "$issue")"
    if [ -z "$meta" ]; then
        skipped=$((skipped + 1))
        rm -f "$hb"
        unset "LAST_NUDGE_TS[$issue]"
        continue
    fi

    state="${meta%% *}"
    labels="${meta#* }"
    in_progress="false"
    if printf '%s' ",${labels}," | grep -q ",in-progress-by-bot,"; then
        in_progress="true"
    fi

    if [ "$state" != "OPEN" ] || [ "$in_progress" != "true" ]; then
        skipped=$((skipped + 1))
        rm -f "$hb"
        unset "LAST_NUDGE_TS[$issue]"
        continue
    fi

    if [ "$age" -ge "$WATCHDOG_RECLAIM_SECS" ]; then
        reclaim_issue "$issue" "$age"
        reclaimed=$((reclaimed + 1))
        unset "LAST_NUDGE_TS[$issue]"
        rm -f "$hb"
        continue
    fi

    last_nudge="${LAST_NUDGE_TS[$issue]:-0}"
    since_last_nudge=$((now_ts - last_nudge))
    if [ "$last_nudge" -eq 0 ] || [ "$since_last_nudge" -ge "$WATCHDOG_NUDGE_COOLDOWN_SECS" ]; then
        if nudge_issue "$issue"; then
            nudged=$((nudged + 1))
            LAST_NUDGE_TS["$issue"]="$now_ts"
        else
            skipped=$((skipped + 1))
        fi
    else
        skipped=$((skipped + 1))
    fi
done

save_state
printf 'service-watch: nudged=%s reclaimed=%s skipped=%s\n' "$nudged" "$reclaimed" "$skipped"
