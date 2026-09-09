#!/usr/bin/env bash
# scripts/lib/autospec-process-wait.sh — shared "wait for this work" and
# "stop this work" helpers (issue #3938).
#
# Waiting for work by command-line pattern (pgrep/pkill with the -f flag)
# matches the waiter's own argv whenever the pattern appears in it: a
# wrapper whose argv contains the pattern waits for itself to exit and
# hangs forever. These helpers never pattern-match. They operate on the
# exact pid (or the worker's exit-sentinel file), resolved once at entry,
# and every bounded wait carries a deadline: on expiry the helper reports
# the unmet condition and the elapsed seconds to stderr, then returns 1.
#
# Public helpers (source this file, then call):
#
#   autospec_wait_pid <pid> [deadline_secs] [label]
#       Wait until the exact pid is gone (dead or already a zombie).
#       deadline_secs 0 makes the wait lifetime-bound: it ends only when
#       the pid exits. Returns 0 when the pid is gone, 1 when the deadline
#       elapsed (report on stderr), 2 on usage error, 3 when asked to wait
#       on pid 0, pid 1, or the caller's own pid (a wait that can never
#       end).
#   autospec_wait_sentinel <file> [deadline_secs] [label]
#       Wait until the worker's exit-sentinel file exists. Same deadline,
#       report, and return-code contract as autospec_wait_pid.
#   autospec_stop_pid <pid> [policy] [grace_ticks]
#       Stop work by pid — never by pattern. Delegates to
#       autospec_kill_tree (scripts/lib/autospec-process-tree.sh); default
#       policy is leader, default grace is 3 ticks.
#
# Deadline default when the argument is omitted: the
# AUTOSPEC_WAIT_PID_DEFAULT_DEADLINE environment variable (seconds,
# default 3600).

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_PROCESS_WAIT_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_PROCESS_WAIT_LIB_LOADED=1

_AUTOSPEC_PW_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# _autospec_pw_alive PID — true while the pid is running (kill -0 succeeds
# and the process is not already an unreaped zombie).
_autospec_pw_alive() {
    local pid="$1" state
    kill -0 "$pid" 2>/dev/null || return 1
    state="$(ps -o state= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
    case "$state" in
        Z*) return 1 ;;
        *)  return 0 ;;
    esac
}

autospec_wait_pid() {
    local pid="${1:-}"
    local label="${3:-pid ${pid:-?}}"
    local deadline="${2:-}"
    case "$pid" in
        ''|*[!0-9]*)
            printf 'autospec_wait_pid: usage: autospec_wait_pid <pid> [deadline_secs] [label]\n' >&2
            return 2 ;;
    esac
    if [ "$pid" -eq 0 ] || [ "$pid" -eq 1 ]; then
        printf 'autospec_wait_pid: refusing to wait on pid %s (system pid)\n' "$pid" >&2
        return 3
    fi
    if [ "$pid" -eq "$$" ]; then
        printf 'autospec_wait_pid: refusing to wait on own pid %s (a wait that can never end)\n' "$pid" >&2
        return 3
    fi
    if [ -z "$deadline" ]; then
        deadline="${AUTOSPEC_WAIT_PID_DEFAULT_DEADLINE:-3600}"
    fi
    case "$deadline" in
        ''|*[!0-9]*)
            printf 'autospec_wait_pid: deadline_secs must be a non-negative integer, got: %s\n' "$deadline" >&2
            return 2 ;;
    esac
    local start now elapsed
    start="$(date +%s)"
    while :; do
        if ! _autospec_pw_alive "$pid"; then
            return 0
        fi
        if [ "$deadline" -ne 0 ]; then
            now="$(date +%s)"
            elapsed=$(( now - start ))
            if [ "$elapsed" -ge "$deadline" ]; then
                printf 'autospec_wait_pid: deadline %ss exceeded; still waiting for: %s (pid %s) after %ss\n' \
                    "$deadline" "$label" "$pid" "$elapsed" >&2
                return 1
            fi
        fi
        sleep 0.5
    done
}

autospec_wait_sentinel() {
    local file="${1:-}"
    local label="${3:-sentinel ${1:-?}}"
    local deadline="${2:-}"
    if [ -z "$file" ]; then
        printf 'autospec_wait_sentinel: usage: autospec_wait_sentinel <file> [deadline_secs] [label]\n' >&2
        return 2
    fi
    if [ -z "$deadline" ]; then
        deadline="${AUTOSPEC_WAIT_PID_DEFAULT_DEADLINE:-3600}"
    fi
    case "$deadline" in
        ''|*[!0-9]*)
            printf 'autospec_wait_sentinel: deadline_secs must be a non-negative integer, got: %s\n' "$deadline" >&2
            return 2 ;;
    esac
    local start now elapsed
    start="$(date +%s)"
    while :; do
        if [ -e "$file" ]; then
            return 0
        fi
        if [ "$deadline" -ne 0 ]; then
            now="$(date +%s)"
            elapsed=$(( now - start ))
            if [ "$elapsed" -ge "$deadline" ]; then
                printf 'autospec_wait_sentinel: deadline %ss exceeded; still waiting for: %s (file %s) after %ss\n' \
                    "$deadline" "$label" "$file" "$elapsed" >&2
                return 1
            fi
        fi
        sleep 0.5
    done
}

autospec_stop_pid() {
    local pid="${1:-}" policy="${2:-leader}" grace="${3:-3}"
    case "$pid" in
        ''|*[!0-9]*)
            printf 'autospec_stop_pid: usage: autospec_stop_pid <pid> [policy] [grace_ticks]\n' >&2
            return 2 ;;
    esac
    if [ "$pid" -eq 0 ] || [ "$pid" -eq 1 ] || [ "$pid" -eq "$$" ]; then
        printf 'autospec_stop_pid: refusing to stop pid %s (system or own pid)\n' "$pid" >&2
        return 3
    fi
    if ! declare -F autospec_kill_tree >/dev/null 2>&1; then
        if [ ! -f "$_AUTOSPEC_PW_LIB_DIR/autospec-process-tree.sh" ]; then
            printf 'autospec_stop_pid: process-tree helper missing: %s\n' \
                "$_AUTOSPEC_PW_LIB_DIR/autospec-process-tree.sh" >&2
            return 2
        fi
        # shellcheck source=autospec-process-tree.sh
        . "$_AUTOSPEC_PW_LIB_DIR/autospec-process-tree.sh"
    fi
    autospec_kill_tree "$pid" "$policy" "$grace"
}
