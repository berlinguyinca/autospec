#!/usr/bin/env bash
# scripts/loop-controller.sh — Self-heal loop controller for autospec-test.
#
# Usage: loop-controller.sh --state-file <path> --gate-cmd <cmd>
#                           [--max-iterations N] [--budget-seconds N]
#                           [--pr-number N] [--worktree <path>]
#
# Drives the self-heal loop:
#   1. Check termination conditions
#   2. Run gate; if passes, exit SUCCESS
#   3. Classify failures
#   4. Pause budget, invoke implementer, resume budget
#   5. Record iteration in state file
#   6. Loop
#
# Exit codes:
#   0 = gate passed
#   1 = loop terminated (budget/iterations/stuck/stop.flag/empty-action)
#   2 = fatal error

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUDGET_SCRIPT="$SCRIPT_DIR/loop-budget.sh"
CLASSIFIER_SCRIPT="$SCRIPT_DIR/loop-classifier.mjs"
STOP_FLAG="${AUTOSPEC_STOP_FLAG:-$HOME/.autospec/stop.flag}"
MAX_SAME_ERROR="${AUTOSPEC_SAME_ERROR_HALT:-3}"
MAX_EMPTY_ACTION="${AUTOSPEC_EMPTY_ACTION_HALT:-2}"

# ── Parse args ────────────────────────────────────────────────────────────────

STATE_FILE=""
GATE_CMD=""
MAX_ITERATIONS="${AUTOSPEC_MAX_LOOP_ITERATIONS:-5}"
BUDGET_SECONDS="${AUTOSPEC_CODING_BUDGET_SECS:-3600}"
PR_NUMBER=0
WORKTREE="${WORKTREE:-$(pwd)}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --state-file)    STATE_FILE="$2";    shift 2 ;;
        --gate-cmd)      GATE_CMD="$2";      shift 2 ;;
        --max-iterations) MAX_ITERATIONS="$2"; shift 2 ;;
        --budget-seconds) BUDGET_SECONDS="$2"; shift 2 ;;
        --pr-number)     PR_NUMBER="$2";     shift 2 ;;
        --worktree)      WORKTREE="$2";      shift 2 ;;
        *) printf 'loop-controller: unknown arg: %s\n' "$1" >&2; exit 2 ;;
    esac
done

if [ -z "$STATE_FILE" ] || [ -z "$GATE_CMD" ]; then
    printf 'Usage: loop-controller.sh --state-file <path> --gate-cmd <cmd>\n' >&2
    exit 2
fi

# ── Initialize or resume state ────────────────────────────────────────────────

# Source budget functions
# shellcheck source=loop-budget.sh
source "$BUDGET_SCRIPT"

if [ ! -f "$STATE_FILE" ]; then
    budget_start "$STATE_FILE" "$PR_NUMBER" "$BUDGET_SECONDS"
    echo "[loop-controller] starting fresh loop (pr=$PR_NUMBER budget=${BUDGET_SECONDS}s max_iter=$MAX_ITERATIONS)"
else
    budget_resume "$STATE_FILE"
    ITER_COUNT=$(jq '.iterations | length' "$STATE_FILE")
    USED=$(jq '.coding_time_used_seconds' "$STATE_FILE")
    echo "[loop-controller] resuming loop (pr=$PR_NUMBER iter=$ITER_COUNT used=${USED}s)"
fi

# ── Helper: update termination_reason in state ────────────────────────────────

set_termination() {
    local reason="$1"
    local tmp
    tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
    jq --arg r "$reason" '.termination_reason = $r' "$STATE_FILE" > "$tmp" && mv "$tmp" "$STATE_FILE"
}

# ── Helper: record iteration ──────────────────────────────────────────────────

record_iteration() {
    local n="$1"
    local classification="$2"
    local gate_passed="$3"
    local error_sig="${4:-null}"
    local started_at="$5"
    local ended_at="$6"

    local tmp
    tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
    jq \
        --argjson n "$n" \
        --arg classification "$classification" \
        --argjson gate_passed "$gate_passed" \
        --arg error_sig "$error_sig" \
        --arg started_at "$started_at" \
        --arg ended_at "$ended_at" \
        '.iterations += [{
            "n": $n,
            "started_at": $started_at,
            "ended_at": $ended_at,
            "classification": $classification,
            "files_changed": [],
            "gate_passed": $gate_passed,
            "error_signature": (if $error_sig == "null" then null else $error_sig end)
        }]' "$STATE_FILE" > "$tmp" && mv "$tmp" "$STATE_FILE"
}

# ── Main loop ─────────────────────────────────────────────────────────────────

# Initialize from state file (for resume semantics)
ITER=0
CONSECUTIVE_SAME_ERROR=$(jq -r '.same_error_consecutive // 0' "$STATE_FILE" 2>/dev/null || echo 0)
CONSECUTIVE_EMPTY_ACTION=$(jq -r '.empty_action_consecutive // 0' "$STATE_FILE" 2>/dev/null || echo 0)
LAST_ERROR_SIG=$(jq -r '.last_error_signature // "null"' "$STATE_FILE" 2>/dev/null || echo "null")

while true; do
    ITER=$(jq '.iterations | length' "$STATE_FILE")
    ITER_NUM=$(( ITER + 1 ))
    ITER_START=$(date -u +'%Y-%m-%dT%H:%M:%SZ')

    # ── Check stop flag ───────────────────────────────────────────────────────
    if [ -f "$STOP_FLAG" ]; then
        echo "[loop-controller] stop.flag detected — exiting gracefully"
        set_termination "stop_flag"
        exit 1
    fi

    # ── Check iteration cap ───────────────────────────────────────────────────
    if [ "$ITER" -ge "$MAX_ITERATIONS" ]; then
        echo "[loop-controller] max iterations ($MAX_ITERATIONS) reached — exiting"
        set_termination "max_iterations"
        exit 1
    fi

    # ── Check budget ──────────────────────────────────────────────────────────
    if budget_exhausted "$STATE_FILE"; then
        echo "[loop-controller] coding budget exhausted — exiting"
        set_termination "budget_exhausted"
        exit 1
    fi

    # ── Check same-error pre-halt (from resumed state) ────────────────────────
    if [ "$CONSECUTIVE_SAME_ERROR" -ge "$MAX_SAME_ERROR" ]; then
        echo "[loop-controller] same error $CONSECUTIVE_SAME_ERROR consecutive times (from state) — halting"
        set_termination "same_error_halt"
        exit 1
    fi

    # ── Check empty-action pre-halt (from resumed state) ─────────────────────
    if [ "$CONSECUTIVE_EMPTY_ACTION" -ge "$MAX_EMPTY_ACTION" ]; then
        echo "[loop-controller] empty action $CONSECUTIVE_EMPTY_ACTION consecutive times (from state) — halting"
        set_termination "empty_action_halt"
        exit 1
    fi

    REMAINING=$(budget_remaining "$STATE_FILE")
    echo "[loop-controller] iteration $ITER_NUM/$MAX_ITERATIONS (remaining budget: ${REMAINING}s)"

    # ── Run gate ──────────────────────────────────────────────────────────────
    GATE_OUTPUT_FILE=$(mktemp /tmp/loop-gate-XXXXXX)
    # Inline cleanup via variable (not trap, per project memory feedback_bash_return_trap_leak)

    GATE_EXIT=0
    budget_pause "$STATE_FILE"
    # Run gate command; capture output
    if eval "$GATE_CMD" > "$GATE_OUTPUT_FILE" 2>/dev/null; then
        GATE_PASSED=true
        GATE_EXIT=0
    else
        GATE_EXIT=$?
        GATE_PASSED=false
    fi
    budget_resume "$STATE_FILE"

    if [ "$GATE_PASSED" = "true" ]; then
        echo "[loop-controller] gate PASSED — loop complete"
        rm -f "$GATE_OUTPUT_FILE"
        set_termination "gate_passed"
        exit 0
    fi

    # ── Extract error signature ───────────────────────────────────────────────
    GATE_JSON=$(cat "$GATE_OUTPUT_FILE" 2>/dev/null || echo '{}')
    STDERR_TEXT=$(printf '%s' "$GATE_JSON" | jq -r '.test_run_summary.stderr_tail // ""')
    STDOUT_TEXT=$(printf '%s' "$GATE_JSON" | jq -r '.test_run_summary.stdout_tail // ""')
    ERROR_TEXT="${STDERR_TEXT}${STDOUT_TEXT}"

    if [ -n "$ERROR_TEXT" ] && command -v node >/dev/null 2>&1; then
        ERROR_SIG=$(printf '%s' "$ERROR_TEXT" | node "$SCRIPT_DIR/error-signature.mjs" 2>/dev/null || echo "null")
    else
        ERROR_SIG="null"
    fi

    # ── Check same-error halt ─────────────────────────────────────────────────
    if [ "$ERROR_SIG" != "null" ] && [ "$ERROR_SIG" = "$LAST_ERROR_SIG" ]; then
        CONSECUTIVE_SAME_ERROR=$(( CONSECUTIVE_SAME_ERROR + 1 ))
        # Update state
        local_tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
        jq --argjson n "$CONSECUTIVE_SAME_ERROR" '.same_error_consecutive = $n' \
            "$STATE_FILE" > "$local_tmp" && mv "$local_tmp" "$STATE_FILE"

        if [ "$CONSECUTIVE_SAME_ERROR" -ge "$MAX_SAME_ERROR" ]; then
            echo "[loop-controller] same error $CONSECUTIVE_SAME_ERROR consecutive times — halting"
            set_termination "same_error_halt"
            rm -f "$GATE_OUTPUT_FILE"
            exit 1
        fi
    else
        CONSECUTIVE_SAME_ERROR=0
        LAST_ERROR_SIG="$ERROR_SIG"
        local_tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
        jq --arg sig "$ERROR_SIG" \
           '.last_error_signature = $sig | .same_error_consecutive = 0' \
           "$STATE_FILE" > "$local_tmp" && mv "$local_tmp" "$STATE_FILE"
    fi

    # ── Classify failure ──────────────────────────────────────────────────────
    CLASSIFY_RESULT=$(node "$CLASSIFIER_SCRIPT" \
        --gate-result "$GATE_OUTPUT_FILE" 2>/dev/null \
        || echo '{"classification":"empty_action","target_failures":[],"suggested_files":[],"estimated_minutes":0,"priority":0}')

    CLASSIFICATION=$(printf '%s' "$CLASSIFY_RESULT" | jq -r '.classification')
    SUGGESTED_FILES=$(printf '%s' "$CLASSIFY_RESULT" | jq -r '.suggested_files | join(" ")')

    # ── Check empty-action halt ───────────────────────────────────────────────
    if [ "$CLASSIFICATION" = "empty_action" ]; then
        CONSECUTIVE_EMPTY_ACTION=$(( CONSECUTIVE_EMPTY_ACTION + 1 ))
        local_tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
        jq --argjson n "$CONSECUTIVE_EMPTY_ACTION" '.empty_action_consecutive = $n' \
            "$STATE_FILE" > "$local_tmp" && mv "$local_tmp" "$STATE_FILE"

        if [ "$CONSECUTIVE_EMPTY_ACTION" -ge "$MAX_EMPTY_ACTION" ]; then
            echo "[loop-controller] empty action $CONSECUTIVE_EMPTY_ACTION consecutive times — halting"
            set_termination "empty_action_halt"
            rm -f "$GATE_OUTPUT_FILE"
            exit 1
        fi
    else
        CONSECUTIVE_EMPTY_ACTION=0
        local_tmp=$(mktemp /tmp/loop-ctrl-XXXXXX)
        jq '.empty_action_consecutive = 0' "$STATE_FILE" > "$local_tmp" && mv "$local_tmp" "$STATE_FILE"
    fi

    echo "[loop-controller] iteration $ITER_NUM: classification=$CLASSIFICATION files=$SUGGESTED_FILES"

    # ── Record iteration (before implementer runs) ────────────────────────────
    ITER_END=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
    record_iteration "$ITER_NUM" "$CLASSIFICATION" "false" "$ERROR_SIG" "$ITER_START" "$ITER_END"

    # ── Implementer stub ──────────────────────────────────────────────────────
    # In production, the implementer subagent runs here. In this shell controller,
    # we emit the action plan and exit for the outer monitor to dispatch.
    # The controller is designed to be re-invoked after the implementer commits.
    printf '[loop-controller] action plan:\n'
    printf '  classification: %s\n' "$CLASSIFICATION"
    printf '  suggested_files: %s\n' "$SUGGESTED_FILES"
    printf '  gate_result: %s\n' "$GATE_OUTPUT_FILE"
    printf '[loop-controller] iteration %s complete — re-invoke after implementing fixes\n' "$ITER_NUM"

    rm -f "$GATE_OUTPUT_FILE"

    # In non-CI mode, exit after printing the plan (implementer is external)
    # In loop mode (AUTOSPEC_LOOP=1), continue looping (for testing purposes)
    if [ "${AUTOSPEC_LOOP:-0}" != "1" ]; then
        exit 0
    fi
done
