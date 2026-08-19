#!/usr/bin/env bash
# scripts/lib/autospec-process-tree.sh — shared process-tree kill helper (issue #2751).
#
# Extracts the group-leader-safe kill_tree idiom duplicated across
# autospec-autonomous-{run,verify,explore}-drain.sh and
# scripts/autospec-explore.sh:224 (_explore_kill_tree) into one sourced lib.
#
# Public entry point: autospec_kill_tree <pid> <policy> [grace_ticks]
#   none               - signal only the exact pid (no recursion, no group signal)
#   leader             - recursive PPID-chain descendant walk, individual kill
#   separate           - pid must be its own process-group leader; group-directed kill
#   separate-recursive - separate, plus a leader-style descendant walk for escapees
# grace_ticks: 0.1s poll ticks before escalating TERM->KILL (default 20 = 2s).
# Returns: 0 signalled, 2 bad usage/policy, 3 safety refusal.

if [ -n "${_AUTOSPEC_PROCESS_TREE_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_PROCESS_TREE_LOADED=1

_autospec_pt_pgid() {
    ps -o pgid= -p "$1" 2>/dev/null | tr -d ' '
}

# Refuse pid 0, pid 1, our own pid, and anything sharing our own process group —
# these guards run BEFORE any signal is considered, for every policy.
_autospec_pt_safe_pid() {
    local pid="$1" self_pgid target_pgid
    case "$pid" in ''|*[!0-9]*) return 1 ;; esac
    [ "$pid" -eq 0 ] && return 1
    [ "$pid" -eq 1 ] && return 1
    [ "$pid" -eq "$$" ] && return 1
    self_pgid="$(_autospec_pt_pgid "$$")"
    target_pgid="$(_autospec_pt_pgid "$pid")"
    if [ -n "$self_pgid" ] && [ -n "$target_pgid" ] && [ "$target_pgid" = "$self_pgid" ]; then
        return 1
    fi
    return 0
}

_autospec_pt_escalate() {
    local pid="$1" grace="$2" i=0
    kill -TERM "$pid" 2>/dev/null || return 0
    while [ "$i" -lt "$grace" ]; do
        kill -0 "$pid" 2>/dev/null || return 0
        i=$((i + 1))
        sleep 0.1
    done
    kill -KILL "$pid" 2>/dev/null || true
}

# Post-order PPID-chain walk: kills descendants before the pid itself. Reaches
# anything still linked by parent-child, including a descendant that changed
# its own process group (e.g. via nested setsid) — but NOT a descendant that
# was reparented away (orphaned) before this walk started.
_autospec_pt_descend() {
    local pid="$1" grace="$2" child
    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        _autospec_pt_descend "$child" "$grace"
    done
    _autospec_pt_escalate "$pid" "$grace"
}

# Group-directed signal: reaches every member of the pgid regardless of the
# PPID chain (so it catches orphans reparented within the same group), but
# never a descendant that escaped into its own separate group.
_autospec_pt_group_kill() {
    local pgid="$1" grace="$2" i=0
    kill -TERM -- "-$pgid" 2>/dev/null || return 0
    while [ "$i" -lt "$grace" ]; do
        kill -0 -- "-$pgid" 2>/dev/null || return 0
        i=$((i + 1))
        sleep 0.1
    done
    kill -KILL -- "-$pgid" 2>/dev/null || true
}

autospec_kill_tree() {
    local pid="$1" policy="$2" grace="${3:-20}" pgid
    case "$pid" in ''|*[!0-9]*)
        echo "autospec_kill_tree: invalid pid '$pid'" >&2
        return 2
        ;;
    esac
    if ! _autospec_pt_safe_pid "$pid"; then
        echo "autospec_kill_tree: refusing unsafe pid $pid" >&2
        return 3
    fi
    case "$policy" in
        none)
            _autospec_pt_escalate "$pid" "$grace"
            ;;
        leader)
            _autospec_pt_descend "$pid" "$grace"
            ;;
        separate)
            pgid="$(_autospec_pt_pgid "$pid")"
            if [ -z "$pgid" ] || [ "$pgid" != "$pid" ]; then
                echo "autospec_kill_tree: pid $pid is not its own process-group leader; refusing 'separate'" >&2
                return 3
            fi
            _autospec_pt_group_kill "$pgid" "$grace"
            ;;
        separate-recursive)
            pgid="$(_autospec_pt_pgid "$pid")"
            if [ -z "$pgid" ] || [ "$pgid" != "$pid" ]; then
                echo "autospec_kill_tree: pid $pid is not its own process-group leader; refusing 'separate-recursive'" >&2
                return 3
            fi
            # Walk the still-live PPID chain FIRST (catches nested-setsid
            # escapees while the leader still links to them), then group-kill
            # by pgid (catches orphans; a pgid signal works even after the
            # leader pid itself has already died in the descend above).
            _autospec_pt_descend "$pid" "$grace"
            _autospec_pt_group_kill "$pgid" "$grace"
            ;;
        *)
            echo "autospec_kill_tree: unknown policy '$policy' (want none|leader|separate|separate-recursive)" >&2
            return 2
            ;;
    esac
}
