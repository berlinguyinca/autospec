#!/usr/bin/env bash
# autospec-autonomous-run-drain.sh — one Tier-1 drain invocation for the conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${AUTOSPEC_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
DRAIN_STALL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS:-1800}"
DRAIN_POLL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS:-15}"

if ! command -v omx >/dev/null 2>&1; then
    printf 'autospec-autonomous-run-drain: omx not found on PATH\n' >&2
    exit 127
fi

stat_size() {
    stat -f '%z' /dev/fd/1 2>/dev/null || stat -c '%s' /dev/fd/1 2>/dev/null || printf ''
}

kill_tree() {
    _pid="$1"
    for _child in $(pgrep -P "$_pid" 2>/dev/null || true); do
        kill_tree "$_child"
    done
    kill "$_pid" 2>/dev/null || true
}

omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    '$autospec-run' &
child_pid="$!"

if [ "${DRAIN_STALL_SECS:-0}" -le 0 ] 2>/dev/null; then
    wait "$child_pid"
    exit "$?"
fi

last_size="$(stat_size)"
last_progress_epoch="$(date +%s)"

while kill -0 "$child_pid" 2>/dev/null; do
    sleep "$DRAIN_POLL_SECS"
    current_size="$(stat_size)"
    if [ -n "$current_size" ] && [ "$current_size" != "$last_size" ]; then
        last_size="$current_size"
        last_progress_epoch="$(date +%s)"
        continue
    fi
    now_epoch="$(date +%s)"
    idle_secs=$((now_epoch - last_progress_epoch))
    if [ "$idle_secs" -ge "$DRAIN_STALL_SECS" ]; then
        printf 'autospec-autonomous-run-drain: stalled after %ss with no output; terminating autospec-run child pid %s\n' \
            "$DRAIN_STALL_SECS" "$child_pid" >&2
        kill_tree "$child_pid"
        wait "$child_pid" 2>/dev/null || true
        exit 124
    fi
done

wait "$child_pid"
exit "$?"
