#!/usr/bin/env bash
# scripts/loop-budget.sh — Pausable 60-minute coding-time budget for the self-heal loop.
#
# Usage:
#   source loop-budget.sh          (to use functions in the controller)
#   loop-budget.sh <command> <state_file> [args]
#
# Commands (when called as a script):
#   budget_start  <state_file> <pr_number> [budget_seconds]
#   budget_pause  <state_file>
#   budget_resume <state_file>
#   budget_remaining <state_file>   → prints remaining seconds
#   budget_exhausted <state_file>   → exits 0 if exhausted, 1 if remaining
#
# State file: .autospec/test-loop-state.json in the target worktree.
# All wall-clock operations use date -u +%s for UTC epoch seconds.
#
# Design: coding time = wall clock minus paused intervals.
# Timer pauses while tests run; budget only counts active editing time.

set -eu

# ── Helpers ───────────────────────────────────────────────────────────────────

_now_epoch() {
    date -u +%s
}

_now_iso() {
    date -u +'%Y-%m-%dT%H:%M:%SZ'
}

_epoch_to_iso() {
    local epoch="$1"
    # macOS: date -r; Linux: date -d @
    date -u -r "$epoch" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "@$epoch" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || printf '%sZ' "$(date -u +%Y-%m-%dT%H:%M:%S -d @"$epoch" 2>/dev/null || echo "1970-01-01T00:00:00")"
}

_iso_to_epoch() {
    local iso="$1"
    # Strip trailing Z for macOS date -j (which ignores Z and uses local time without -u)
    local iso_stripped="${iso%Z}"
    # macOS: date -u -j -f (UTC); Linux: date -u -d
    date -u -j -f '%Y-%m-%dT%H:%M:%S' "$iso_stripped" +%s 2>/dev/null \
        || date -u -d "$iso" +%s 2>/dev/null \
        || echo 0
}

# ── budget_start ──────────────────────────────────────────────────────────────

budget_start() {
    local state_file="$1"
    local pr_number="${2:-0}"
    local budget_seconds="${3:-3600}"
    local now_iso
    now_iso=$(_now_iso)
    local now_epoch
    now_epoch=$(_now_epoch)

    jq -n \
        --argjson pr_number "$pr_number" \
        --arg started_at "$now_iso" \
        --argjson budget "$budget_seconds" \
        --arg last_start "$now_iso" \
        '{
            "pr_number": $pr_number,
            "started_at": $started_at,
            "coding_time_used_seconds": 0,
            "coding_time_budget_seconds": $budget,
            "last_coding_start": $last_start,
            "iterations": [],
            "last_error_signature": null,
            "same_error_consecutive": 0,
            "empty_action_consecutive": 0,
            "termination_reason": null
        }' > "$state_file"
}

# ── budget_pause ──────────────────────────────────────────────────────────────
# Called before running tests (pauses the coding-time clock).

budget_pause() {
    local state_file="$1"
    if [ ! -f "$state_file" ]; then
        printf 'loop-budget: error: state file not found: %s\n' "$state_file" >&2
        return 1
    fi

    local last_start now_epoch used
    last_start=$(jq -r '.last_coding_start // empty' "$state_file")
    used=$(jq -r '.coding_time_used_seconds' "$state_file")
    now_epoch=$(_now_epoch)

    if [ -z "$last_start" ] || [ "$last_start" = "null" ]; then
        # Already paused
        return 0
    fi

    local start_epoch elapsed
    start_epoch=$(_iso_to_epoch "$last_start")
    elapsed=$(( now_epoch - start_epoch ))
    local new_used
    new_used=$(echo "$used $elapsed" | awk '{printf "%d", $1 + $2}')

    # Update state: accumulate used time, clear last_coding_start
    local tmp
    tmp=$(mktemp /tmp/loop-budget-XXXXXX.json)
    jq --argjson new_used "$new_used" \
       '.coding_time_used_seconds = $new_used | .last_coding_start = null' \
       "$state_file" > "$tmp" && mv "$tmp" "$state_file"
}

# ── budget_resume ─────────────────────────────────────────────────────────────
# Called after tests complete (resumes the coding-time clock).

budget_resume() {
    local state_file="$1"
    if [ ! -f "$state_file" ]; then
        printf 'loop-budget: error: state file not found: %s\n' "$state_file" >&2
        return 1
    fi

    local now_iso
    now_iso=$(_now_iso)

    local last_start
    last_start=$(jq -r '.last_coding_start // empty' "$state_file")
    if [ -n "$last_start" ] && [ "$last_start" != "null" ]; then
        # Already running
        return 0
    fi

    local tmp
    tmp=$(mktemp /tmp/loop-budget-XXXXXX.json)
    jq --arg now "$now_iso" \
       '.last_coding_start = $now' \
       "$state_file" > "$tmp" && mv "$tmp" "$state_file"
}

# ── budget_remaining ──────────────────────────────────────────────────────────
# Prints remaining coding seconds (accounting for any running window).

budget_remaining() {
    local state_file="$1"
    if [ ! -f "$state_file" ]; then
        echo 0; return
    fi

    local budget used last_start
    budget=$(jq -r '.coding_time_budget_seconds // 3600' "$state_file")
    used=$(jq -r '.coding_time_used_seconds // 0' "$state_file")
    last_start=$(jq -r '.last_coding_start // empty' "$state_file")

    local current_elapsed=0
    if [ -n "$last_start" ] && [ "$last_start" != "null" ]; then
        local now_epoch start_epoch
        now_epoch=$(_now_epoch)
        start_epoch=$(_iso_to_epoch "$last_start")
        current_elapsed=$(( now_epoch - start_epoch ))
    fi

    local total_used remaining
    total_used=$(echo "$used $current_elapsed" | awk '{printf "%d", $1 + $2}')
    remaining=$(echo "$budget $total_used" | awk '{r = $1 - $2; print (r > 0 ? r : 0)}')
    echo "$remaining"
}

# ── budget_exhausted ──────────────────────────────────────────────────────────
# Exits 0 if budget is exhausted, 1 if still remaining.

budget_exhausted() {
    local state_file="$1"
    local remaining
    remaining=$(budget_remaining "$state_file")
    [ "$remaining" -eq 0 ]
}

# ── CLI dispatch ──────────────────────────────────────────────────────────────

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    CMD="${1:-}"
    STATE_FILE="${2:-}"
    shift 2 2>/dev/null || true

    case "$CMD" in
        budget_start)    budget_start "$STATE_FILE" "$@" ;;
        budget_pause)    budget_pause "$STATE_FILE" ;;
        budget_resume)   budget_resume "$STATE_FILE" ;;
        budget_remaining) budget_remaining "$STATE_FILE" ;;
        budget_exhausted) budget_exhausted "$STATE_FILE"; exit $? ;;
        *)
            printf 'Usage: loop-budget.sh <command> <state_file> [args]\n' >&2
            printf 'Commands: budget_start budget_pause budget_resume budget_remaining budget_exhausted\n' >&2
            exit 1
            ;;
    esac
fi
