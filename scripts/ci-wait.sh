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
# State values: pending | pass | fail | stalled | died
#   died = the poller stopped without a verdict (signalled, failed, or
#   killed without running its exit trap). The poller's exit trap settles
#   the sentinel to died and appends a `ci-wait: terminal:` line to the
#   log naming the outcome (completed / failed / signalled + signal name).
#
# Use ci-wait-poll.sh <PR> to read the signal (exit 0/1/2/3/4).
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

# Spawn background poller via setsid (#4094): a bare `nohup ... &` child
# shares the caller's process group, so a timeout or Ctrl-C in the calling
# context kills the poller mid-run and leaves no trace. In its own session
# the poller's pid is also its pgid/sid, and setsid does not fork a
# non-group-leader child, so $! below is the poller itself.
setsid bash -c '
PR="'"$PR"'"
TIMEOUT='"$TIMEOUT"'
REQUIRED_ONLY='"$REQUIRED_ONLY"'
SIGNAL_FILE="'"$SIGNAL_FILE"'"
PID_FILE="'"$PID_FILE"'"
NOTIFY_SH="'"$NOTIFY_SH"'"

# Prove we started, from inside the session we own: the reader checks this
# pid for liveness, never the log mtime (#4094 AC3, the #3995 rule applied
# to the pass itself).
printf "%s\n" "$$" > "$PID_FILE"

write_signal() {
    local state="$1"
    local checks_json="${2:-[]}"
    local settled_at
    settled_at="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '"'"'{"pr":"%s","state":"%s","checks":%s,"settled_at":"%s"}\n'"'"' \
        "$PR" "$state" "$checks_json" "$settled_at" > "$SIGNAL_FILE"
}

# Terminal line on every exit — completed, failed, or signalled, with the
# signal named (#4094 AC2). A log that stops without a terminal line is a
# detectable defect, never an ambiguous "still running".
write_terminal() {
    local rc="$1" kind sig state ts
    if [ "$rc" -gt 127 ] 2>/dev/null; then
        kind="signalled"
        sig="$(kill -l "$((rc - 128))" 2>/dev/null || printf UNKNOWN)"
    elif [ "$rc" -eq 0 ]; then
        kind="completed"
    else
        kind="failed"
    fi
    state="$(jq -r '"'"'.state // "pending"'"'"' "$SIGNAL_FILE" 2>/dev/null || printf pending)"
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ "$state" = "pending" ]; then
        # We stopped without a verdict: settle the sentinel to died so a
        # reader never has to guess "running" from a dead poller.
        printf '"'"'{"pr":"%s","state":"died","checks":[],"settled_at":"%s"}\n'"'"' \
            "$PR" "$ts" > "$SIGNAL_FILE"
        state="died"
    fi
    if [ "$kind" = "signalled" ]; then
        printf '"'"'ci-wait: terminal: %s %s state=%s signal=%s rc=%s\n'"'"' \
            "$kind" "$ts" "$state" "$sig" "$rc"
    else
        printf '"'"'ci-wait: terminal: %s %s state=%s rc=%s\n'"'"' \
            "$kind" "$ts" "$state" "$rc"
    fi
}

# Bash does not pass the exit status of a caught signal to the EXIT trap, so
# convert catchable signals to their conventional 128+N exit codes first; the
# EXIT trap then names the signal in the terminal line.
trap '"'"'rc=$?; write_terminal "$rc"; exit "$rc"'"'"' EXIT
trap '"'"'exit 129'"'"' HUP
trap '"'"'exit 130'"'"' INT
trap '"'"'exit 143'"'"' TERM

START="$(date +%s)"
elapsed() { printf "%s" "$(( $(date +%s) - START ))"; }

while true; do
    # Fetch status check rollup
    rollup="$(gh pr view "$PR" --json statusCheckRollup --jq ".statusCheckRollup // []" 2>/dev/null || printf "[]")"

    total="$(printf "%s" "$rollup" | jq "length" 2>/dev/null || printf "0")"
    bad="$(printf "%s" "$rollup" | jq "[.[] | select(.conclusion==\"FAILURE\" or .conclusion==\"CANCELLED\" or .conclusion==\"TIMED_OUT\" or .conclusion==\"ACTION_REQUIRED\")] | length" 2>/dev/null || printf "0")"
    # gh reports in-progress checks with conclusion "" (not null), so pending
    # must key off status alone: any check not COMPLETED is still running.
    pending="$(printf "%s" "$rollup" | jq "[.[] | select(.status != \"COMPLETED\")] | length" 2>/dev/null || printf "0")"

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

# Publish the pending sentinel only after the poller's pid is on record, so
# a reader that sees state=pending can always check liveness (#4094 AC3).
printf '{"pr":"%s","state":"pending","checks":[],"settled_at":null}\n' "$PR" > "$SIGNAL_FILE"

printf 'ci-wait: PR #%s — poller spawned (PID %s), signal at %s\n' "$PR" "$POLLER_PID" "$SIGNAL_FILE"
