#!/usr/bin/env bash
# ci-wait-poll.sh — Read the CI-wait sentinel signal file for a PR.
#
# Usage:
#   bash scripts/ci-wait-poll.sh <PR>
#
# Exit codes:
#   0  state=pass    (CI settled, all checks green)
#   1  state=fail    (CI settled, at least one check failed)
#   1  state=stalled (CI did not settle before timeout)
#   2  state=pending (sentinel exists, CI not yet settled, poller alive)
#   3  sentinel missing (call ci-wait.sh first)
#   4  state=died    (poller is not alive — no terminal line to trust)
#
# "pending" is only ever reported for a poller whose PID is alive. Liveness is
# read from the poller's own process (kill -0 on the recorded PID), never from
# the log's last-write time — the #3995 rule applied to the pass itself
# (#4094 AC3). A sentinel that says pending but whose poller is gone means the
# poller died without settling (e.g. SIGKILL skips even the exit trap), so the
# reader reports "died" and exits 4 instead of guessing "running".
#
# Stdout: the state string (pass|fail|stalled|pending|died)

set -euo pipefail

if [ $# -lt 1 ]; then
    printf 'Usage: ci-wait-poll.sh <PR>\n' >&2
    exit 3
fi

PR="$1"
CI_STATE_DIR="${HOME}/.autospec/ci-state"
SIGNAL_FILE="${CI_STATE_DIR}/${PR}.signal"
PID_FILE="${CI_STATE_DIR}/${PR}.pid"

if [ ! -f "$SIGNAL_FILE" ]; then
    printf 'ci-wait-poll: no sentinel for PR %s (call ci-wait.sh first)\n' "$PR" >&2
    exit 3
fi

state="$(jq -r '.state // "pending"' "$SIGNAL_FILE" 2>/dev/null || printf 'pending')"

if [ "$state" = "pending" ]; then
    # Liveness check: the sentinel alone cannot tell "running" from "died
    # without a terminal line". Ask the poller's own process.
    pid=""
    if [ -f "$PID_FILE" ]; then
        pid="$(tr -d '[:space:]' < "$PID_FILE" 2>/dev/null || printf '')"
    fi
    if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
        state="died"
    fi
fi

printf '%s\n' "$state"

case "$state" in
    pass)    exit 0 ;;
    fail)    exit 1 ;;
    stalled) exit 1 ;;
    pending) exit 2 ;;
    died)    exit 4 ;;
    *)       exit 2 ;;
esac
