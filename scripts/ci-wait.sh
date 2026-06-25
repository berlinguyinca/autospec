#!/usr/bin/env bash
# ci-wait.sh — Fire-and-forget CI-wait sentinel.
# Spawns a background poller that writes a JSON signal file when CI settles.
# Returns immediately so the calling agent does not block.
#
# Usage:
#   bash scripts/ci-wait.sh <PR> [--timeout SECONDS] [--required-only]
#
# Outputs:
#   ~/.autospec/ci-state/<PR>.signal  — JSON: {pr, state, checks, settled_at}
#   ~/.autospec/ci-state/<PR>.pid     — PID of background poller
#   ~/.autospec/ci-state/<PR>.log     — poller stdout/stderr
#
# State values: pending | pass | fail | stalled
#
# Use ci-wait-poll.sh <PR> to read the signal (exit 0/1/2/3).
# Use ci-wait-cleanup.sh <PR> to stop the poller and remove files.
#
# Exit codes:
#   0  Poller spawned successfully
#   1  Missing required argument

set -euo pipefail

if [ $# -lt 1 ]; then
    printf 'Usage: ci-wait.sh <PR> [--timeout SECONDS] [--required-only]\n' >&2
    exit 1
fi

PR="$1"
shift

TIMEOUT=1800  # 30 minutes default
REQUIRED_ONLY=0

while [ $# -gt 0 ]; do
    case "$1" in
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --required-only)
            REQUIRED_ONLY=1
            shift
            ;;
        *)
            printf 'ci-wait.sh: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

CI_STATE_DIR="${HOME}/.autospec/ci-state"
mkdir -p "$CI_STATE_DIR"

SIGNAL_FILE="${CI_STATE_DIR}/${PR}.signal"
PID_FILE="${CI_STATE_DIR}/${PR}.pid"
LOG_FILE="${CI_STATE_DIR}/${PR}.log"

# Resolve shared notify helper relative to this script's repo root.
# skills/autospec-shared/scripts/notify.sh is the canonical entry point
# (mirrors the osascript/notify-send pattern from the context-monitor adapter).
_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NOTIFY_SH="${_SCRIPT_DIR}/../skills/autospec-shared/scripts/notify.sh"

# Remove stale signal/PID from a previous run
rm -f "$SIGNAL_FILE" "$PID_FILE"

# Write initial pending signal so callers know we started
printf '{"pr":"%s","state":"pending","checks":[],"settled_at":null}\n' "$PR" > "$SIGNAL_FILE"

# Spawn background poller via nohup
nohup bash -c '
PR="'"$PR"'"
TIMEOUT='"$TIMEOUT"'
REQUIRED_ONLY='"$REQUIRED_ONLY"'
SIGNAL_FILE="'"$SIGNAL_FILE"'"
NOTIFY_SH="'"$NOTIFY_SH"'"

write_signal() {
    local state="$1"
    local checks_json="${2:-[]}"
    local settled_at
    settled_at="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '"'"'{"pr":"%s","state":"%s","checks":%s,"settled_at":"%s"}\n'"'"' \
        "$PR" "$state" "$checks_json" "$settled_at" > "$SIGNAL_FILE"
}

START="$(date +%s)"
elapsed() { printf "%s" "$(( $(date +%s) - START ))"; }

while true; do
    # Fetch status check rollup
    rollup="$(gh pr view "$PR" --json statusCheckRollup --jq ".statusCheckRollup // []" 2>/dev/null || printf "[]")"

    total="$(printf "%s" "$rollup" | jq "length" 2>/dev/null || printf "0")"
    bad="$(printf "%s" "$rollup" | jq "[.[] | select(.conclusion==\"FAILURE\" or .conclusion==\"CANCELLED\" or .conclusion==\"TIMED_OUT\" or .conclusion==\"ACTION_REQUIRED\")] | length" 2>/dev/null || printf "0")"
    pending="$(printf "%s" "$rollup" | jq "[.[] | select(.conclusion == null and .status != \"COMPLETED\")] | length" 2>/dev/null || printf "0")"

    if [ "$bad" -gt 0 ]; then
        write_signal "fail" "$rollup"
        # Notify once on terminal verdict — failure must never block the merge path.
        bash "$NOTIFY_SH" "autospec: CI failed" "PR #${PR} — one or more checks failed" || true
        exit 0
    fi

    if [ "$pending" -eq 0 ] && [ "$total" -gt 0 ]; then
        write_signal "pass" "$rollup"
        bash "$NOTIFY_SH" "autospec: CI passed" "PR #${PR} — all checks green" || true
        exit 0
    fi

    if [ "$(elapsed)" -gt "$TIMEOUT" ]; then
        write_signal "stalled" "$rollup"
        bash "$NOTIFY_SH" "autospec: CI stalled" "PR #${PR} — timed out after ${TIMEOUT}s" || true
        exit 0
    fi

    sleep 30
done
' > "$LOG_FILE" 2>&1 &

POLLER_PID=$!
printf '%s\n' "$POLLER_PID" > "$PID_FILE"

printf 'ci-wait: PR #%s — poller spawned (PID %s), signal at %s\n' "$PR" "$POLLER_PID" "$SIGNAL_FILE"
