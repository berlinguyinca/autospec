#!/usr/bin/env bash
# scripts/autospec-timeout.sh — one bounded-run helper for every gh / git / ssh
# call in the watchdog and disk-hygiene scripts. Sourceable; runs nothing.
#
#   autospec_timeout <secs> <command> [args...]
#       Runs the command with a time limit; exit 124 on timeout. Uses
#       timeout / gtimeout when installed, else autospec_timeout_fallback.
#   autospec_timeout_fallback <secs> <command> [args...]
#       Pure bash (stock macOS has no timeout): run in the background, a
#       watcher sleeps then kills the command AND its descendants (so a git
#       fetch's ssh child cannot keep a `$(...)` capture open), exit 124.
#
# Conventions: bash 3.2 safe, no RETURN traps.

_at_kill_tree() { # $1 pid: TERM the descendants, then the pid itself
    local child
    for child in $(pgrep -P "$1" 2>/dev/null); do
        _at_kill_tree "$child"
    done
    kill -TERM "$1" 2>/dev/null || true
}

autospec_timeout_fallback() {
    local secs="$1" pid watcher rc
    shift
    "$@" &
    pid=$!
    ( sleep "$secs"; _at_kill_tree "$pid"; sleep 2; kill -KILL "$pid" 2>/dev/null ) >/dev/null 2>&1 &
    watcher=$!
    wait "$pid" 2>/dev/null
    rc=$?
    _at_kill_tree "$watcher"
    wait "$watcher" 2>/dev/null || true
    # 143/137: killed by the watcher's TERM/KILL
    case "$rc" in 143|137) return 124 ;; esac
    return "$rc"
}

autospec_timeout() {
    local secs="$1" t
    shift
    for t in timeout gtimeout; do
        if command -v "$t" >/dev/null 2>&1; then
            "$t" "$secs" "$@"
            return $?
        fi
    done
    autospec_timeout_fallback "$secs" "$@"
}
