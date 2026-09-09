#!/usr/bin/env bash
# scripts/lib/autospec-log-status.sh — single-sample observation guards
# (issue #3973).
#
# A log is an append-only source in motion: a result line not yet written
# is "not yet", not "not ever". A single sample of such a source describes
# a moment; a conclusion (stalled, broken, done) describes a state, and
# absence at time T is not evidence of absence full stop. These helpers
# make the state claim the only claim a reader can make, and keep the
# observation (sizes, timestamps, both samples) separate from the
# inference (the verdict) in every report:
#
#   autospec_log_status <logfile> [terminal_marker] [gap_seconds]
#       Verdict from markers and two samples, never from recency alone.
#       A terminal marker (default: `######## complete ########`) is
#       final by definition, so one observation of it suffices → complete.
#       Without a marker the log is sampled, the reader waits
#       <gap_seconds> (default 2), and samples again: the log grew, or the
#       marker appeared in the meantime → running; two identical samples →
#       stalled. `stalled` is only ever produced by two independent
#       observations — a single sample can never yield it. The observation
#       evidence (both samples with sizes, digests, and timestamps) is
#       printed to stderr under `obs:`; the verdict goes to stdout under
#       an `inference:` line on stderr plus the bare word on stdout.
#       Returns 0 with a verdict, 1 when the log is missing or unreadable,
#       2 on usage.
#   autospec_log_gate <logfile> [terminal_marker] [gap_seconds]
#       Destructive-action gate (stop, re-dispatch, archive). Allows (0)
#       only on complete, or on a stalled verdict that already carries two
#       independent observations; refuses (1) on running — a live system
#       is never acted destructively on on one glance at its log. Passes
#       status' observation lines through. Returns 0/1 as above, 2 on
#       usage, 1 when the log is unreadable.
#   autospec_log_count <logfile> <pattern>
#       A computed count (fixed-string, whole log) of <pattern>. A
#       judgement expressible as a number is computed, not estimated from
#       the visible tail. Prints the count (0 included — a computed zero
#       is a result, not an error); returns 0, 1 when the log is missing
#       or unreadable, 2 on usage.
#   autospec_log_terminal <logfile> [marker]
#       Append the terminal completion marker (default:
#       `######## complete ########`) so a reader can ASK the log whether
#       the pass is finished instead of inferring it from silence.
#   autospec_log_heartbeat <logfile> [step]
#       Append a heartbeat line (`heartbeat: <step> <UTC timestamp>`) so
#       "no output yet" stays distinguishable from "stopped" while the
#       step is still between writes.

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_LOG_STATUS_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_LOG_STATUS_LIB_LOADED=1

_AUTOSPEC_LOG_DEFAULT_MARKER='######## complete ########'

# _autospec_log_sample FILE — print one line "<size> <sha256>" for FILE.
# The digest, not just the size: a rotated or truncated log can land on
# the same byte count it had before.
_autospec_log_sample() {
    local file="$1" size digest
    size="$(wc -c < "$file" 2>/dev/null | tr -d '[:space:]')" || return 1
    digest="$(sha256sum "$file" 2>/dev/null | cut -d' ' -f1)" || return 1
    [ -n "$size" ] && [ -n "$digest" ] || return 1
    printf '%s %s' "$size" "$digest"
}

_autospec_log_usable() {
    local file="$1"
    [ -f "$file" ] && [ -r "$file" ]
}

autospec_log_status() {
    local file="${1:-}" marker="${2:-}" gap="${3:-2}"
    # An omitted (or empty) marker means "the default marker": grep -F ''
    # would match every line and short-circuit the verdict to complete.
    [ -n "$marker" ] || marker="$_AUTOSPEC_LOG_DEFAULT_MARKER"
    if [ -z "$file" ] || [ -z "$gap" ]; then
        printf 'autospec_log_status: usage: autospec_log_status <logfile> [terminal_marker] [gap_seconds]\n' >&2
        return 2
    fi
    case "$gap" in
        ''|*[!0-9]*)
            printf 'autospec_log_status: gap_seconds must be a non-negative integer: %s\n' "$gap" >&2
            return 2
            ;;
    esac
    if ! _autospec_log_usable "$file"; then
        printf 'autospec_log_status: cannot read log: %s\n' "$file" >&2
        return 1
    fi

    # A terminal marker is final: one observation of it is enough.
    if grep -qF -- "$marker" "$file"; then
        printf 'inference: complete (terminal marker present)\n' >&2
        printf 'complete\n'
        return 0
    fi

    local s1 s2 ts1 ts2
    s1="$(_autospec_log_sample "$file")" || return 1
    ts1="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    sleep "$gap"

    # The pass may have written its marker while we were waiting.
    if grep -qF -- "$marker" "$file"; then
        ts2="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'obs: no marker at %s (%s); terminal marker present at %s\n' \
            "$ts1" "$s1" "$ts2" >&2
        printf 'inference: complete (terminal marker appeared within the gap)\n' >&2
        printf 'complete\n'
        return 0
    fi

    s2="$(_autospec_log_sample "$file")" || return 1
    ts2="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ "$s1" != "$s2" ]; then
        printf 'obs: sample 1 @ %s: %s; sample 2 @ %s: %s (log advanced)\n' \
            "$ts1" "$s1" "$ts2" "$s2" >&2
        printf 'inference: running (two samples differ)\n' >&2
        printf 'running\n'
        return 0
    fi
    # Two identical samples, a gap apart: the only verdict absence
    # supports is "no progress over this interval" — and only because it
    # was observed twice.
    printf 'obs: sample 1 @ %s: %s; sample 2 @ %s: %s (no change over %ss)\n' \
        "$ts1" "$s1" "$ts2" "$s2" "$gap" >&2
    printf 'inference: stalled (two independent observations)\n' >&2
    printf 'stalled\n'
    return 0
}

autospec_log_gate() {
    local file="${1:-}"
    if [ -z "$file" ]; then
        printf 'autospec_log_gate: usage: autospec_log_gate <logfile> [terminal_marker] [gap_seconds]\n' >&2
        return 2
    fi
    local verdict rc
    verdict="$(autospec_log_status "$@")"
    rc=$?
    if [ "$rc" -ne 0 ]; then
        return "$rc"
    fi
    case "$verdict" in
        complete|stalled)
            printf 'autospec_log_gate: allowing — verdict %s is marker-backed or double-observed\n' \
                "$verdict" >&2
            return 0
            ;;
        running)
            printf 'autospec_log_gate: REFUSING — log is running; a destructive action is never taken on one glance at a live system\n' >&2
            return 1
            ;;
        *)
            printf 'autospec_log_gate: unknown verdict: %s\n' "$verdict" >&2
            return 2
            ;;
    esac
}

autospec_log_count() {
    local file="${1:-}" pattern="${2:-}"
    if [ -z "$file" ] || [ -z "$pattern" ]; then
        printf 'autospec_log_count: usage: autospec_log_count <logfile> <pattern>\n' >&2
        return 2
    fi
    if ! _autospec_log_usable "$file"; then
        printf 'autospec_log_count: cannot read log: %s\n' "$file" >&2
        return 1
    fi
    local n
    # grep -c exits 1 on zero matches but still prints 0; a computed
    # zero is a result, so the function returns 0 either way.
    n="$(grep -cF -- "$pattern" "$file")" || n=0
    printf '%s\n' "$n"
    return 0
}

autospec_log_terminal() {
    local file="${1:-}" marker="${2:-$_AUTOSPEC_LOG_DEFAULT_MARKER}"
    if [ -z "$file" ]; then
        printf 'autospec_log_terminal: usage: autospec_log_terminal <logfile> [marker]\n' >&2
        return 2
    fi
    printf '%s\n' "$marker" >> "$file" || return 1
    return 0
}

autospec_log_heartbeat() {
    local file="${1:-}" step="${2:-pipeline}"
    if [ -z "$file" ]; then
        printf 'autospec_log_heartbeat: usage: autospec_log_heartbeat <logfile> [step]\n' >&2
        return 2
    fi
    printf 'heartbeat: %s %s\n' "$step" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$file" || return 1
    return 0
}
