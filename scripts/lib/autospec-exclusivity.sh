#!/usr/bin/env bash
# scripts/lib/autospec-exclusivity.sh — held-lock exclusivity and pidfile
# identity helpers (issue #3967).
#
# A process count is a sample, not a guarantee: counting competitors at
# startup is a check-then-act race (it establishes that no competitor
# existed *once*, then acts as though none can appear), and a count taken
# from inside the process table includes the measurer (issue #3963, #3938).
# Exclusivity is a property you hold, not a property you observe. These
# helpers acquire it:
#
#   autospec_gate <lockfile> [label]
#       Take an flock on <lockfile>, held on a dedicated fd for the
#       lifetime of this shell (released by the kernel on exit, crash
#       included). Refuses (returns 1) while a competitor holds it. The
#       lockfile also carries a human-readable holder note (pid/label/
#       since) written after the lock is taken — for observation only,
#       never the control.
#   autospec_gate_release
#       Drop the lock before the shell exits (optional; the fd closing on
#       exit releases it anyway).
#   autospec_gate_advisory <pattern> [label]
#       Warning only, never a gate: reports how many running processes
#       match <pattern> (via autospec_match_procs) as a WARN line. Always
#       returns 0 (2 on usage).
#   autospec_spawn <pidfile> <cmd> [args...]
#       Run <cmd> detached (setsid) and write the leader pid to <pidfile>
#       (mode 0600). The pidfile, not the process table, is the identity
#       a later run consults. The worker's stdin/stdout/stderr go to
#       /dev/null — a long-running worker must not inherit the caller's
#       fds, or a pipe write end left open would block the caller's
#       output capture until the worker exits (wrap the command to log).
#       Prints the pid on stdout.
#   autospec_pidfile_pid <pidfile>
#       Print the live pid recorded in <pidfile>. Returns 0 with the pid,
#       1 when the file is missing, unreadable, malformed, or the pid is
#       no longer alive, 2 on usage. Compose with autospec_wait_pid /
#       autospec_stop_pid (scripts/lib/autospec-process-wait.sh).
#   autospec_match_procs <pattern>
#       The pattern-matching fallback, last resort only. Snapshots the
#       process table once (so the snapshot pipeline can never match
#       itself), then matches the fixed string <pattern> against each
#       command line. Excludes the matcher's whole session — itself,
#       every ancestor, and every live descendant: all of them carry the
#       pattern in their argv whenever the command being run IS the
#       search (the wrapper shells and pipeline processes of issue
#       #3967). Prints every match — "<pid>: <args>" — to stderr BEFORE
#       any caller can act on the list, and the matching pids to stdout.
#       Returns 0 with matches, 1 with none, 2 on usage.
#
# Command-line matchers (pgrep/pkill -f) are rejected in repo scripts by
# scripts/lint-process-matchers.sh (issue #3938); this lib is the approved
# replacement.

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_EXCLUSIVITY_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_EXCLUSIVITY_LIB_LOADED=1

# ── Held-lock exclusivity ────────────────────────────────────────────────────

autospec_gate() {
    local lockfile="${1:-}" label="${2:-exclusive gate}"
    if [ -z "$lockfile" ]; then
        printf 'autospec_gate: usage: autospec_gate <lockfile> [label]\n' >&2
        return 2
    fi
    # Open read-write without truncating: a contender must not clobber the
    # holder's note while probing. bash picks a free fd on its own.
    # shellcheck disable=SC2054 # dynamic-fd assignment into a named variable
    # NOTE: no `2>/dev/null` here — an exec redirection persists for the
    # rest of the shell, so it would swallow every later stderr message
    # (including the REFUSING line below).
    if ! exec {_AUTOSPEC_EX_GATE_FD}<> "$lockfile"; then
        printf 'autospec_gate: cannot open lockfile: %s\n' "$lockfile" >&2
        return 2
    fi
    # shellcheck disable=SC2054 # fd is a variable by design
    if ! flock -n "$_AUTOSPEC_EX_GATE_FD"; then
        # Advisory observation only: who is holding it, if the note is there.
        local holder
        holder="$(head -n1 "$lockfile" 2>/dev/null || true)"
        printf 'autospec_gate: REFUSING — %s is already held (lockfile: %s%s)\n' \
            "$label" "$lockfile" \
            "${holder:+; holder: $holder}" >&2
        # Close the probe fd by number: `exec {var}>&-` would assign a
        # NEW free fd to the variable and close that, leaking this one.
        # shellcheck disable=SC2086 # fd number by design
        { eval "exec $_AUTOSPEC_EX_GATE_FD>&-"; } 2>/dev/null || true
        return 1
    fi
    # We own the lock now: truncate the stale note and write our own.
    : > "$lockfile"
    printf 'pid=%s host=%s label=%s since=%s\n' \
        "$BASHPID" "$(hostname 2>/dev/null || true)" "$label" \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >&"$_AUTOSPEC_EX_GATE_FD"
    return 0
}

autospec_gate_release() {
    local fd="${_AUTOSPEC_EX_GATE_FD:-}"
    case "$fd" in
        ''|*[!0-9]*) return 0 ;;
    esac
    # shellcheck disable=SC2054
    flock -u "$fd" 2>/dev/null || true
    # Close by number (see autospec_gate: `exec {var}>&-` would close a
    # different, freshly-assigned fd).
    # shellcheck disable=SC2086 # fd number by design
    { eval "exec $fd>&-"; } 2>/dev/null || true
    _AUTOSPEC_EX_GATE_FD=""
    return 0
}

autospec_gate_advisory() {
    local pattern="${1:-}" label="${2:-competitor scan}"
    if [ -z "$pattern" ]; then
        printf 'autospec_gate_advisory: usage: autospec_gate_advisory <pattern> [label]\n' >&2
        return 2
    fi
    local pids count
    pids="$(autospec_match_procs "$pattern" 2>/dev/null)"
    if [ -n "$pids" ]; then
        count="$(printf '%s\n' "$pids" | wc -l | tr -d '[:space:]')"
        printf 'WARN: %d process(es) matched "%s" (%s); advisory only — exclusivity comes from autospec_gate, not from this count\n' \
            "$count" "$pattern" "$label" >&2
    fi
    return 0
}

# ── Pidfile identity ─────────────────────────────────────────────────────────

autospec_spawn() {
    local pidfile="${1:-}"
    shift
    if [ -z "$pidfile" ] || [ "$#" -lt 1 ]; then
        printf 'autospec_spawn: usage: autospec_spawn <pidfile> <cmd> [args...]\n' >&2
        return 2
    fi
    local dir
    dir="$(dirname -- "$pidfile")"
    [ -d "$dir" ] || mkdir -p -- "$dir" || return 2
    # Detach stdio: see the header note on why the worker must not keep
    # the caller's fds open.
    setsid "$@" </dev/null >/dev/null 2>&1 &
    local pid=$!
    # 0600: the pidfile is identity, not public status.
    ( umask 077; printf '%s\n' "$pid" > "$pidfile" ) || return 2
    printf '%s\n' "$pid"
    return 0
}

autospec_pidfile_pid() {
    local pidfile="${1:-}"
    if [ -z "$pidfile" ]; then
        printf 'autospec_pidfile_pid: usage: autospec_pidfile_pid <pidfile>\n' >&2
        return 2
    fi
    [ -f "$pidfile" ] && [ -r "$pidfile" ] || return 1
    local pid
    pid="$(tr -d '[:space:]' < "$pidfile" 2>/dev/null)" || return 1
    case "$pid" in
        ''|*[!0-9]*) return 1 ;;
    esac
    kill -0 "$pid" 2>/dev/null || return 1
    printf '%s\n' "$pid"
    return 0
}

# ── Pattern-match fallback (last resort) ─────────────────────────────────────

# _autospec_ex_descendants PID — print every live descendant of PID
# (identity-based pgrep -P walk, never a pattern match).
_autospec_ex_descendants() {
    local pid="$1" child
    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        printf '%s\n' "$child"
        _autospec_ex_descendants "$child"
    done
}

# _autospec_ex_session_set OUT_VAR — fill OUT_VAR (newline-separated) with
# the matcher's own session: itself (BASHPID, so a $(...) subshell still
# names itself correctly, where $$ still names the parent), every
# ancestor up to pid 1, and every live descendant.
_autospec_ex_session_set() {
    local outvar="$1" pid="${BASHPID:-$$}" parent list
    list="$pid"
    while :; do
        parent="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
        case "$parent" in
            ''|0|1) break ;;
        esac
        if printf '%s\n' "$list" | grep -qx -- "$parent"; then
            # Cycle guard: a pid that repeats means we walked in a loop.
            break
        fi
        pid="$parent"
        list="$list
$parent"
    done
    list="$list
$(_autospec_ex_descendants "${BASHPID:-$$}")"
    printf -v "$outvar" '%s\n' "$list"
}

autospec_match_procs() {
    local pattern="${1:-}"
    if [ -z "$pattern" ]; then
        printf 'autospec_match_procs: usage: autospec_match_procs <pattern>\n' >&2
        return 2
    fi
    local exclude pid args matches=""
    _autospec_ex_session_set exclude || return 2
    # Snapshot to a file: a $(ps ...) subshell would itself carry the
    # caller's argv — and therefore the pattern — into the very table it
    # is measuring. A plain redirect spawns no subshell, and the ps
    # process's own argv ("ps -eo pid=,args=") cannot match the pattern.
    local table_file
    table_file="$(mktemp "${TMPDIR:-/tmp}/autospec-ps.XXXXXX")" || return 2
    if ! ps -eo pid=,args= > "$table_file" 2>/dev/null; then
        rm -f -- "$table_file"
        return 2
    fi
    local line
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        # read splits the leading pid off; the rest (spaces preserved) is
        # the full command line.
        read -r pid args <<< "$line"
        case "$pid" in
            ''|*[!0-9]*) continue ;;
        esac
        if [[ "$args" != *"$pattern"* ]]; then
            continue
        fi
        if printf '%s\n' "$exclude" | grep -qx -- "$pid"; then
            printf 'autospec_match_procs: excluded (matching session): %s: %s\n' \
                "$pid" "$args" >&2
            continue
        fi
        printf 'autospec_match_procs: match: %s: %s\n' "$pid" "$args" >&2
        matches="${matches}${pid}
"
    done < "$table_file"
    rm -f -- "$table_file"
    if [ -z "$matches" ]; then
        return 1
    fi
    printf '%s\n' "$matches" | sed '/^$/d'
    return 0
}
