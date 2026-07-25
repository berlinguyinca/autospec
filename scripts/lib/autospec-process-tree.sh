#!/usr/bin/env bash
# Shared process-tree reaper for detached autospec harnesses.
#
# autospec_kill_tree <pid> [separate|leader|none] [grace-seconds]
#   separate: signal a process group that differs from the caller's group.
#   leader:   signal the group only when pid is its group leader.
#   none:     recurse through descendants without group signalling.

if [ -n "${_AUTOSPEC_PROCESS_TREE_LOADED:-}" ]; then
    return 0 2>/dev/null || true
fi
_AUTOSPEC_PROCESS_TREE_LOADED=1

autospec_kill_tree() {
    local pid="$1"
    local group_policy="${2:-separate}"
    local grace_seconds="${3:-0}"
    local child pgid="" own_pgid="" group_owned=0

    case "$group_policy" in
        separate)
            pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
            own_pgid="$(ps -o pgid= -p "$$" 2>/dev/null | tr -d ' ' || true)"
            [ -n "$pgid" ] && [ "$pgid" != "$own_pgid" ] && group_owned=1
            ;;
        leader)
            pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
            [ -n "$pgid" ] && [ "$pgid" = "$pid" ] && group_owned=1
            ;;
        none) ;;
        *) return 2 ;;
    esac

    if [ "$group_owned" -eq 1 ]; then
        kill -TERM -- "-$pgid" 2>/dev/null || true
        if [ "$group_policy" = "separate" ]; then
            kill -KILL -- "-$pgid" 2>/dev/null || true
            return 0
        fi
    fi

    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        autospec_kill_tree "$child" "$group_policy" "$grace_seconds"
    done
    kill -TERM "$pid" 2>/dev/null || true
    # The autonomous explore/verify drains historically used TERM-only when a
    # target shared the caller's process group. Preserve that cleanup window;
    # only an owned detached group is safe for their immediate KILL escalation.
    if [ "$group_policy" = "separate" ]; then
        return 0
    fi
    if [ "$grace_seconds" -gt 0 ] 2>/dev/null; then
        sleep "$grace_seconds"
    fi
    if [ "$group_owned" -eq 1 ]; then
        kill -KILL -- "-$pgid" 2>/dev/null || true
    fi
    kill -KILL "$pid" 2>/dev/null || true
}
