#!/usr/bin/env bash
# ci-wait-cleanup.sh — Stop the CI-wait background poller and remove sentinel files.
#
# Usage:
#   bash scripts/ci-wait-cleanup.sh <PR>
#
# Kills the PID from <PR>.pid (if alive), then removes <PR>.{pid,signal,log}.
# Safe to call even if files are missing or poller already exited.
#
# Exit codes:
#   0  Cleanup complete

set -euo pipefail

if [ $# -lt 1 ]; then
    printf 'Usage: ci-wait-cleanup.sh <PR>\n' >&2
    exit 1
fi

PR="$1"
CI_STATE_DIR="${HOME}/.autospec/ci-state"
PID_FILE="${CI_STATE_DIR}/${PR}.pid"
SIGNAL_FILE="${CI_STATE_DIR}/${PR}.signal"
LOG_FILE="${CI_STATE_DIR}/${PR}.log"

# Kill background poller if PID file exists
if [ -f "$PID_FILE" ]; then
    poller_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [ -n "$poller_pid" ]; then
        kill "$poller_pid" 2>/dev/null || true
    fi
fi

# Remove all sentinel files (safe if missing)
rm -f "$PID_FILE" "$SIGNAL_FILE" "$LOG_FILE"

printf 'ci-wait-cleanup: PR #%s sentinel files removed\n' "$PR"
