#!/usr/bin/env bash
# Shared process-tree termination for watchdogs and detached harnesses.

_autospec_process_tree_pids() {
    local pid="$1" child

    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        _autospec_process_tree_pids "$child"
    done
    printf '%s\n' "$pid"
}

autospec_kill_process_tree() {
    local pid="${1:-}" grace_seconds="${2:-1}"
    local pgid own_pgid targets tree_pid

    case "$pid" in
        ''|*[!0-9]*) return 0 ;;
    esac
    case "$grace_seconds" in
        ''|*[!0-9]*) grace_seconds=1 ;;
    esac

    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    own_pgid="$(ps -o pgid= -p "$$" 2>/dev/null | tr -d ' ' || true)"

    # A setsid child owns its process group. Signal that group directly so
    # detached grandchildren cannot outlive the wrapper. Otherwise recurse by
    # parent PID and never risk signalling the caller's process group.
    if [ -n "$pgid" ] && [ "$pgid" = "$pid" ] && [ "$pgid" != "$own_pgid" ]; then
        kill -TERM -- "-$pgid" 2>/dev/null || true
        [ "$grace_seconds" -eq 0 ] || sleep "$grace_seconds"
        kill -KILL -- "-$pgid" 2>/dev/null || true
        return 0
    fi

    targets="$(_autospec_process_tree_pids "$pid")"
    for tree_pid in $targets; do
        kill -TERM "$tree_pid" 2>/dev/null || true
    done
    [ "$grace_seconds" -eq 0 ] || sleep "$grace_seconds"
    for tree_pid in $targets; do
        kill -KILL "$tree_pid" 2>/dev/null || true
    done
}
