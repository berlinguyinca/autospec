#!/usr/bin/env bash
# scripts/lib/autospec-log-state.sh — moment-vs-state helpers (issue #3973).
#
# A single observation of an append-only or asynchronous source describes a
# moment, not a state: "no result line yet" is not "no result will come",
# and "no dispatch in the tail" is not "dispatch is broken". Six confident
# diagnoses made that mistake in one session, and one of them killed a
# job that had actually opened 86 PRs. This lib makes the distinction
# mechanical so no reader has to infer it:
#
# Writers (AC: every long-running step emits a heartbeat and a terminal
# marker):
#
#   autospec_heartbeat <logfile> [text]
#       Append one heartbeat line (timestamp + pid).
#   autospec_terminal <logfile> [status]
#       Append the terminal marker line `######## <status> ########`
#       (default status: `complete`; e.g. `failed rc=7` on error).
#   autospec_run_step <logfile> [--heartbeat-every SECS] [--] <cmd> [args...]
#       Run <cmd> while a heartbeat is appended every SECS (default 30);
#       on exit append the terminal marker (complete, or failed rc=N) and
#       propagate the command's exit code.
#
# Readers (state comes from a terminal record or from TWO independent
# observations — never from log recency alone):
#
#   autospec_log_state <logfile> [--marker RE] [--gap SECS]
#       `complete` (rc 3): a line matches the terminal marker (default
#       RE `^######## .+ ########$`) — a terminal record, one observation
#       suffices. Otherwise the log is sampled (sha256) twice, gap SECS
#       apart (default 2): different → `running` (rc 0), identical →
#       `stalled` (rc 4). A freshly written but quiet log is stalled, not
#       running — recency is not evidence of motion. rc 1: log missing or
#       unreadable; rc 2: usage.
#   autospec_destructive_guard <logfile> [--marker RE] [--gap SECS]
#       Gate for stop / re-dispatch / archive. Returns 0 (allowed) only on
#       a terminal marker or on the two-sample stall verdict; returns 1
#       (refusing) while running, and fails closed on any other error.
#   autospec_log_yield <logfile> <pattern>
#       A yield/throughput characterisation is a count, computed over the
#       WHOLE file, not an estimate from the visible tail: prints
#       `grep -c <pattern>` over <logfile>. rc 0 (even at count 0), 1:
#       unreadable, 2: usage.

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_LOG_STATE_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_LOG_STATE_LIB_LOADED=1

_AUTOSPEC_LS_DEFAULT_MARKER='^######## .+ ########$'

# _autospec_ls_parse_state_args — shared option parsing for
# autospec_log_state / autospec_destructive_guard. Sets the globals
# _AUTOSPEC_LS_LOG, _AUTOSPEC_LS_MARKER, _AUTOSPEC_LS_GAP. Returns 2 on
# usage error.
_autospec_ls_parse_state_args() {
    _AUTOSPEC_LS_LOG=""
    _AUTOSPEC_LS_MARKER="$_AUTOSPEC_LS_DEFAULT_MARKER"
    _AUTOSPEC_LS_GAP=2
    local positional=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --marker)
                _AUTOSPEC_LS_MARKER="${2:-}"
                shift 2 || return 2
                ;;
            --gap)
                _AUTOSPEC_LS_GAP="${2:-}"
                shift 2 || return 2
                ;;
            --)
                shift
                break
                ;;
            -*)
                return 2
                ;;
            *)
                [ "$positional" -eq 0 ] || return 2
                _AUTOSPEC_LS_LOG="$1"
                positional=1
                shift
                ;;
        esac
    done
    [ -n "$_AUTOSPEC_LS_LOG" ] || return 2
    [ -n "$_AUTOSPEC_LS_MARKER" ] || return 2
    case "$_AUTOSPEC_LS_GAP" in
        ''|*[!0-9.]*|*.*.*) return 2 ;;
    esac
    return 0
}

autospec_heartbeat() {
    local logfile="${1:-}"
    if [ $# -gt 1 ]; then shift; fi
    local text="$*"
    if [ -z "$logfile" ]; then
        printf 'autospec_heartbeat: usage: autospec_heartbeat <logfile> [text]\n' >&2
        return 2
    fi
    local dir
    dir="$(dirname -- "$logfile")"
    [ -d "$dir" ] || mkdir -p -- "$dir" || return 1
    printf 'autospec-heartbeat %s pid=%s%s\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${BASHPID:-$$}" \
        "${text:+ $text}" >> "$logfile" || return 1
    return 0
}

autospec_terminal() {
    local logfile="${1:-}" status="${2:-complete}"
    if [ -z "$logfile" ]; then
        printf 'autospec_terminal: usage: autospec_terminal <logfile> [status]\n' >&2
        return 2
    fi
    case "$status" in
        ''|*$'\n'*) return 2 ;;
    esac
    local dir
    dir="$(dirname -- "$logfile")"
    [ -d "$dir" ] || mkdir -p -- "$dir" || return 1
    printf '######## %s ########\n' "$status" >> "$logfile" || return 1
    return 0
}

autospec_run_step() {
    local logfile="${1:-}"
    if [ $# -gt 0 ]; then shift; fi
    local every=30
    while [ $# -gt 0 ]; do
        case "$1" in
            --heartbeat-every)
                every="${2:-}"
                shift 2 || break
                ;;
            --)
                shift
                break
                ;;
            *)
                break
                ;;
        esac
    done
    if [ -z "$logfile" ] || [ $# -lt 1 ]; then
        printf 'autospec_run_step: usage: autospec_run_step <logfile> [--heartbeat-every SECS] [--] <cmd> [args...]\n' >&2
        return 2
    fi
    case "$every" in
        ''|*[!0-9.]*|*.*.*)
            printf 'autospec_run_step: --heartbeat-every takes a number of seconds: %s\n' "$every" >&2
            return 2
            ;;
    esac
    # The loop reaps itself if this shell dies: a heartbeat appender that
    # outlived its step would be a lie in the log.
    local hb
    (
        parent=$PPID
        while kill -0 "$parent" 2>/dev/null; do
            sleep "$every"
            autospec_heartbeat "$logfile" "step=$parent"
        done
    ) &
    hb=$!
    local rc=0
    "$@" || rc=$?
    kill "$hb" 2>/dev/null || true
    wait "$hb" 2>/dev/null || true
    if [ "$rc" -eq 0 ]; then
        autospec_terminal "$logfile"
    else
        autospec_terminal "$logfile" "failed rc=$rc"
    fi
    return "$rc"
}

autospec_log_state() {
    _autospec_ls_parse_state_args "$@" || {
        printf 'autospec_log_state: usage: autospec_log_state <logfile> [--marker RE] [--gap SECS]\n' >&2
        return 2
    }
    local logfile="$_AUTOSPEC_LS_LOG"
    # grep exits 0/1 on match/no-match and 2 on a bad regex: only 2 is
    # a usage error.
    local grc=0
    printf '\n' | grep -qE -- "$_AUTOSPEC_LS_MARKER" 2>/dev/null || grc=$?
    if [ "$grc" -eq 2 ]; then
        printf 'autospec_log_state: invalid marker regex: %s\n' "$_AUTOSPEC_LS_MARKER" >&2
        return 2
    fi
    if [ ! -r "$logfile" ]; then
        printf 'autospec_log_state: cannot read log: %s\n' "$logfile" >&2
        return 1
    fi
    # A terminal marker is a record, not a moment: one observation of it
    # is final.
    if grep -qE -- "$_AUTOSPEC_LS_MARKER" "$logfile"; then
        printf 'complete\n'
        return 3
    fi
    # No marker: a single sample cannot separate "not yet" from "not
    # ever", so take two, gap-seconds apart, and compare.
    local s1 s2
    s1="$(sha256sum < "$logfile")"
    sleep "$_AUTOSPEC_LS_GAP"
    s2="$(sha256sum < "$logfile")"
    if [ "$s1" != "$s2" ]; then
        printf 'running\n'
        return 0
    fi
    printf 'stalled\n'
    return 4
}

autospec_destructive_guard() {
    _autospec_ls_parse_state_args "$@" || {
        printf 'autospec_destructive_guard: usage: autospec_destructive_guard <logfile> [--marker RE] [--gap SECS]\n' >&2
        return 2
    }
    local logfile="$_AUTOSPEC_LS_LOG"
    local state
    state="$(autospec_log_state "$logfile" --marker "$_AUTOSPEC_LS_MARKER" --gap "$_AUTOSPEC_LS_GAP")"
    local rc=$?
    case "$state" in
        running)
            printf 'autospec_destructive_guard: REFUSING — %s is running (two samples %ss apart differ); stop/re-dispatch/archive needs a terminal marker\n' \
                "$logfile" "$_AUTOSPEC_LS_GAP" >&2
            return 1
            ;;
        complete)
            printf 'autospec_destructive_guard: ALLOWED — terminal marker present in %s\n' "$logfile" >&2
            return 0
            ;;
        stalled)
            printf 'autospec_destructive_guard: ALLOWED — no terminal marker and two samples %ss apart agree %s has stopped\n' \
                "$logfile" "$_AUTOSPEC_LS_GAP" "$logfile" >&2
            return 0
            ;;
        *)
            # Missing/unreadable log, or an error: fail closed.
            printf 'autospec_destructive_guard: FAIL-CLOSED — state of %s could not be established; gather a second observation before acting\n' \
                "$logfile" >&2
            return 1
            ;;
    esac
}

autospec_log_yield() {
    local logfile="${1:-}" pattern="${2:-}"
    if [ -z "$logfile" ] || [ -z "$pattern" ]; then
        printf 'autospec_log_yield: usage: autospec_log_yield <logfile> <pattern>\n' >&2
        return 2
    fi
    if [ ! -r "$logfile" ]; then
        printf 'autospec_log_yield: cannot read log: %s\n' "$logfile" >&2
        return 1
    fi
    # Count over the whole file, not the visible tail. grep exits 1 at
    # count 0 — zero is a valid answer, not an error.
    local count
    count="$(grep -c -- "$pattern" "$logfile" || true)"
    printf '%s\n' "$count"
    return 0
}
