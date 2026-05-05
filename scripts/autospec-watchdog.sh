#!/usr/bin/env bash
# autospec-watchdog.sh — reclaim and nudge stalled autospec workers.
#
# The monitor should call this at startup, before candidate selection, and on
# its regular service-watch cadence to reconcile `process-heartbeats/*.json`
# files and detect stalled workers.
#
# Environment overrides:
#   AUTOSPEC_WATCHDOG_DIR              heartbeat directory (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_WATCHDOG_REPO              override repo for gh calls (default: gh repo context)
#   AUTOSPEC_WATCHDOG_STALE_SECS         stale threshold (default: 1800)
#   AUTOSPEC_WATCHDOG_RECLAIM_SECS       reclaim threshold (default: 10800)
#   AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS claimed-step release threshold (default: 300)
#   AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS nudge cooldown (default: 900)
#   AUTOSPEC_WATCHDOG_STATE_FILE         state file for nudge cooldown (default: ~/.autospec/watchdog-state.tsv)

set -eu

WATCHDOG_DIR="${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}"
WATCHDOG_REPO="${AUTOSPEC_WATCHDOG_REPO:-${AUTOSPEC_REPO:-}}"
WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
WATCHDOG_CLAIMED_TIMEOUT_SECS="${AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS:-300}"
WATCHDOG_NUDGE_COOLDOWN_SECS="${AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS:-900}"
STATE_FILE="${AUTOSPEC_WATCHDOG_STATE_FILE:-$HOME/.autospec/watchdog-state.tsv}"

WATCHDOG_LOG_PREFIX="[autospec-watchdog]"

if ! command -v gh >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX ERROR: gh CLI not found" >&2
    exit 1
fi

if [ ! -d "$WATCHDOG_DIR" ]; then
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0"
    exit 0
fi

now_ts="$(date -u +%s)"
nudged=0
reclaimed=0
claimed_released=0
garbage_collected=0
invalid_schema=0
skipped=0

if [ -n "$WATCHDOG_REPO" ]; then
    REPO_ARGS="--repo $WATCHDOG_REPO"
else
    REPO_ARGS=""
fi

STATE_LINES=""

load_state() {
    if [ -f "$STATE_FILE" ]; then
        STATE_LINES="$(awk '$1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { print $1 "\t" $2 }' "$STATE_FILE")"
    fi
}

state_get() {
    printf '%s\n' "$STATE_LINES" | awk -v issue="$1" '$1 == issue { print $2; exit }'
}

state_set() {
    issue="$1"
    ts="$2"
    STATE_LINES="$(printf '%s\n' "$STATE_LINES" | awk -v issue="$issue" 'NF && $1 != issue { print }')"
    if [ -n "$STATE_LINES" ]; then
        STATE_LINES="${STATE_LINES}
${issue}	${ts}"
    else
        STATE_LINES="${issue}	${ts}"
    fi
}

state_unset() {
    issue="$1"
    STATE_LINES="$(printf '%s\n' "$STATE_LINES" | awk -v issue="$issue" 'NF && $1 != issue { print }')"
}

issue_meta() {
    # shellcheck disable=SC2086
    gh issue view "$1" $REPO_ARGS \
        --json state,labels \
        --jq '.state + " " + ([.labels[].name] | join(","))' \
        2>/dev/null || true
}

reclaim_issue() {
    local issue="$1"
    local age="$2"

    # shellcheck disable=SC2086
    gh issue edit "$issue" $REPO_ARGS \
        --remove-label in-progress-by-bot \
        --add-label auto-implement >/dev/null 2>&1 || true
    # shellcheck disable=SC2086
    gh issue comment "$issue" $REPO_ARGS \
        --body "autospec watchdog reclaimed this issue after ${age}s of no check-in." >/dev/null 2>&1 || true
}

nudge_issue() {
    local issue="$1"

    # shellcheck disable=SC2086
    gh issue comment "$issue" $REPO_ARGS \
        --body "autospec watchdog: please check in; if stuck, post blocker and clear in-progress-by-bot." \
        >/dev/null 2>&1 || return 1
}

save_state() {
    mkdir -p "$HOME/.autospec"
    if [ -z "$STATE_LINES" ]; then
        rm -f "$STATE_FILE"
        return
    fi
    tmp="$(mktemp "$HOME/.autospec/.watchdog-state.XXXXXX")"
    printf '%s\n' "$STATE_LINES" > "$tmp"
    mv "$tmp" "$STATE_FILE"
}

json_value() {
    key="$1"
    file="$2"
    jq -r --arg key "$key" '.[$key] // empty' "$file" 2>/dev/null || true
}

heartbeat_schema_valid() {
    file="$1"
    issue="$2"

    hb_issue="$(json_value issue "$file")"
    hb_step="$(json_value step "$file")"
    hb_ts="$(json_value ts "$file")"

    case "$hb_issue" in
        "$issue") ;;
        *) return 1 ;;
    esac
    case "$hb_ts" in
        ''|*[!0-9]*) return 1 ;;
    esac
    case "$hb_step" in
        claimed|worktree_ready|tests_started|tests_passed|pr_created|smoke_retry|reviewed|merged|failed) ;;
        *) return 1 ;;
    esac
    return 0
}

normalize_heartbeat() {
    file="$1"
    issue="$2"

    branch="$(json_value branch "$file")"
    step="$(json_value step "$file")"
    ts="$(json_value ts "$file")"
    pr="$(json_value pr "$file")"
    repo="$(json_value repo "$file")"
    tmp="${file}.tmp"
    jq -n \
        --arg issue "$issue" \
        --arg branch "$branch" \
        --arg step "$step" \
        --argjson ts "$ts" \
        --arg pr "$pr" \
        --arg repo "$repo" \
        '{issue:$issue,branch:$branch,step:$step,ts:$ts,pr:$pr,repo:$repo}' > "$tmp" \
        && mv "$tmp" "$file"
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

    meta="$(issue_meta "$issue")"
    if [ -z "$meta" ]; then
        skipped=$((skipped + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    state="${meta%% *}"
    labels="${meta#* }"
    in_progress="false"
    if printf '%s' ",${labels}," | grep -q ",in-progress-by-bot,"; then
        in_progress="true"
    fi

    if [ "$state" != "OPEN" ] || [ "$in_progress" != "true" ]; then
        garbage_collected=$((garbage_collected + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    if ! heartbeat_schema_valid "$hb" "$issue"; then
        invalid_schema=$((invalid_schema + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    normalize_heartbeat "$hb" "$issue"
    ts="$(json_value ts "$hb")"
    step="$(json_value step "$hb")"

    age=$(( now_ts - ts ))
    if [ "$age" -lt 0 ]; then
        age=0
    fi

    if [ "$step" = "claimed" ] && [ "$age" -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then
        reclaim_issue "$issue" "$age"
        claimed_released=$((claimed_released + 1))
        state_unset "$issue"
        rm -f "$hb"
        continue
    fi

    if [ "$age" -lt "$WATCHDOG_STALE_SECS" ]; then
        continue
    fi

    if [ "$age" -ge "$WATCHDOG_RECLAIM_SECS" ]; then
        reclaim_issue "$issue" "$age"
        reclaimed=$((reclaimed + 1))
        state_unset "$issue"
        rm -f "$hb"
        continue
    fi

    last_nudge="$(state_get "$issue")"
    last_nudge="${last_nudge:-0}"
    since_last_nudge=$((now_ts - last_nudge))
    if [ "$last_nudge" -eq 0 ] || [ "$since_last_nudge" -ge "$WATCHDOG_NUDGE_COOLDOWN_SECS" ]; then
        if nudge_issue "$issue"; then
            nudged=$((nudged + 1))
            state_set "$issue" "$now_ts"
        else
            skipped=$((skipped + 1))
        fi
    else
        skipped=$((skipped + 1))
    fi
done

save_state
printf 'service-watch: nudged=%s reclaimed=%s claimed_released=%s garbage_collected=%s invalid_schema=%s skipped=%s\n' \
    "$nudged" "$reclaimed" "$claimed_released" "$garbage_collected" "$invalid_schema" "$skipped"
