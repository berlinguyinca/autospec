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
#   autospec_log_sweep [-w window_seconds] <pattern> <logfile> [logfile...]
#       Error sweep over a SET of log files that reads mtime with every
#       tail (issue #4246). `tail`/`grep` answer "what was written last",
#       which is a different question from "what is happening", and the two
#       diverge exactly when a component has stopped — the case under
#       investigation. A supervisor error log that is only ever appended to
#       on failure shows its last failure, arbitrarily far in the past, as
#       though it were the present. So each file gets a header line with its
#       mtime, age, freshness and match count printed alongside its matches
#       (files with no matches included — a clean result needs a freshness
#       too), and any match from a file older than the investigation window
#       (default 3600s, override with -w) is preceded by a plain `STALE:`
#       line saying the match is history, not current state, and that live
#       state should be queried before an outage is declared. A file whose
#       mtime cannot be read is `freshness=unknown` and its matches count as
#       stale: freshness the sweep cannot prove is never freshness it may
#       claim. Verdicts: `clean` (no matches, every file read), `incomplete`
#       (no matches but at least one file unreadable — a gap, not a clean
#       result), `history-only` (matches only outside the window), `live` (at
#       least one match inside it). Returns 0 on `clean`, `incomplete` or
#       `history-only`, 1 on `live`, 2 on usage. Unreadable files are reported
#       as `freshness=unreadable` and counted in the summary.

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_LOG_STATUS_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_LOG_STATUS_LIB_LOADED=1

_AUTOSPEC_LOG_DEFAULT_MARKER='######## complete ########'
_AUTOSPEC_LOG_SWEEP_DEFAULT_WINDOW=3600

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

# _autospec_log_mtime_epoch FILE — modification time as a unix epoch
# (GNU `stat -c %Y` first, BSD `stat -f %m` fallback); 1 when unreadable.
_autospec_log_mtime_epoch() {
    local file="$1"
    stat -c %Y "$file" 2>/dev/null || stat -f %m "$file" 2>/dev/null || return 1
}

# _autospec_log_iso_epoch EPOCH — the epoch as an RFC3339Z UTC timestamp
# (GNU `date -d @…` first, BSD `date -r …` fallback).
_autospec_log_iso_epoch() {
    local epoch="$1"
    date -u -d "@$epoch" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
        || date -u -r "$epoch" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
        || return 1
}

# _autospec_log_format_age SECONDS — coarse human age (3d, 5h, 12m, 45s).
_autospec_log_format_age() {
    local s="${1:-0}"
    [ "$s" -ge 0 ] 2>/dev/null || s=0
    if [ "$s" -ge 86400 ]; then printf '%dd' "$((s / 86400))"
    elif [ "$s" -ge 3600 ]; then printf '%dh' "$((s / 3600))"
    elif [ "$s" -ge 60 ]; then printf '%dm' "$((s / 60))"
    else printf '%ds' "$s"
    fi
}

# _autospec_log_freshness_of FILE NOW WINDOW — prints "<iso> <age> <freshness>"
# with freshness in fresh|stale|unknown. A file whose mtime cannot be read
# yields `unknown`: freshness the sweep cannot prove is never freshness it
# may claim. A future mtime (clock skew) is fresh, never a negative age.
_autospec_log_freshness_of() {
    local file="$1" now="$2" window="$3" mtime iso age freshness
    mtime="$(_autospec_log_mtime_epoch "$file")" || {
        printf 'unknown unknown unknown\n'
        return 0
    }
    iso="$(_autospec_log_iso_epoch "$mtime")" || iso="$mtime"
    age=$((now - mtime))
    [ "$age" -ge 0 ] || age=0
    if [ "$age" -le "$window" ]; then freshness='fresh'; else freshness='stale'; fi
    printf '%s %s %s\n' "$iso" "$age" "$freshness"
}

# _autospec_log_stale_note FILE AGE ISO WINDOW N — the callout that keeps a
# match from a stale log being read as current state.
_autospec_log_stale_note() {
    local file="$1" age="$2" iso="$3" window="$4" n="$5" human
    case "$age" in
        ''|*[!0-9]*) human='mtime unknown' ;;
        *) human="$(_autospec_log_format_age "$age") ago" ;;
    esac
    printf '%s STALE: last written %s (mtime %s) — outside the %ss investigation window; these %s match(es) are history, not current state. Query live state before declaring an outage.\n' \
        "$file" "$human" "$iso" "$window" "$n"
}

autospec_log_sweep() {
    local window="$_AUTOSPEC_LOG_SWEEP_DEFAULT_WINDOW"
    if [ "${1:-}" = "-w" ]; then
        if [ "$#" -lt 2 ]; then
            printf 'autospec_log_sweep: -w needs window_seconds\n' >&2
            return 2
        fi
        window="$2"
        shift 2
    fi
    case "$window" in
        ''|*[!0-9]*)
            printf 'autospec_log_sweep: window_seconds must be a non-negative integer: %s\n' "$window" >&2
            return 2
            ;;
    esac
    local pattern="${1:-}"
    if [ -z "$pattern" ]; then
        printf 'autospec_log_sweep: usage: autospec_log_sweep [-w window_seconds] <pattern> <logfile> [logfile...]\n' >&2
        return 2
    fi
    shift
    if [ "$#" -lt 1 ]; then
        printf 'autospec_log_sweep: usage: autospec_log_sweep [-w window_seconds] <pattern> <logfile> [logfile...]\n' >&2
        return 2
    fi

    local now
    now="$(date -u +%s)"
    local total=0 unreadable=0 fresh_hits=0 stale_hits=0
    local file iso age freshness hits rc n
    for file in "$@"; do
        total=$((total + 1))
        if ! _autospec_log_usable "$file"; then
            printf '%s freshness=unreadable matches=unknown\n' "$file"
            unreadable=$((unreadable + 1))
            continue
        fi
        IFS=' ' read -r iso age freshness \
            <<< "$(_autospec_log_freshness_of "$file" "$now" "$window")"

        hits="$(grep -nF -- "$pattern" "$file" 2>/dev/null)" && rc=0 || rc=$?
        if [ "$rc" -gt 1 ]; then
            printf '%s freshness=%s matches=unknown\n' "$file" "$freshness"
            unreadable=$((unreadable + 1))
            continue
        fi
        if [ -z "$hits" ]; then
            n=0
        else
            n="$(printf '%s\n' "$hits" | wc -l | tr -d '[:space:]')"
        fi
        printf '%s mtime=%s age=%s freshness=%s matches=%s\n' \
            "$file" "$iso" "$age" "$freshness" "$n"
        if [ "$n" -gt 0 ]; then
            if [ "$freshness" = 'fresh' ]; then
                fresh_hits=$((fresh_hits + n))
            else
                stale_hits=$((stale_hits + n))
                _autospec_log_stale_note "$file" "$age" "$iso" "$window" "$n"
            fi
            printf '%s\n' "$hits"
        fi
    done

    local verdict
    if [ "$fresh_hits" -gt 0 ]; then
        verdict='live'
    elif [ "$stale_hits" -gt 0 ]; then
        verdict='history-only'
    elif [ "$unreadable" -gt 0 ]; then
        # Nothing matched, but part of the sweep could not be read: an
        # unreadable file is a gap, never a clean result.
        verdict='incomplete'
    else
        verdict='clean'
    fi
    printf 'sweep: %s file(s), %s match(es): %s fresh, %s stale, %s unreadable; verdict: %s\n' \
        "$total" "$((fresh_hits + stale_hits))" "$fresh_hits" "$stale_hits" "$unreadable" "$verdict"
    [ "$verdict" = 'live' ] && return 1
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
