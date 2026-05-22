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
#   2  state=pending (sentinel exists but CI not yet settled)
#   3  sentinel missing (call ci-wait.sh first)
#
# Stdout: the state string (pass|fail|stalled|pending)

set -euo pipefail

if [ $# -lt 1 ]; then
    printf 'Usage: ci-wait-poll.sh <PR>\n' >&2
    exit 3
fi

PR="$1"
CI_STATE_DIR="${HOME}/.autospec/ci-state"
SIGNAL_FILE="${CI_STATE_DIR}/${PR}.signal"

if [ ! -f "$SIGNAL_FILE" ]; then
    printf 'ci-wait-poll: no sentinel for PR %s (call ci-wait.sh first)\n' "$PR" >&2
    exit 3
fi

state="$(jq -r '.state // "pending"' "$SIGNAL_FILE" 2>/dev/null || printf 'pending')"
printf '%s\n' "$state"

case "$state" in
    pass)    exit 0 ;;
    fail)    exit 1 ;;
    stalled) exit 1 ;;
    pending) exit 2 ;;
    *)       exit 2 ;;
esac
