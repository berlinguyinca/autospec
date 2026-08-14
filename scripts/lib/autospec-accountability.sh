#!/usr/bin/env bash

_autospec_conductor_accountability_event() {
    local kind="$1" what="$2" why="$3" evidence="$4" project="${5:-0}"
    local repo="${_AUTOSPEC_CONDUCTOR_REPO:-}"
    local bin="${_AUTOSPEC_CONDUCTOR_ACCOUNTABILITY_BIN:-}"
    local required="${AUTOSPEC_ACCOUNTABILITY_REQUIRED:-0}"
    if [ -z "$repo" ] || [ -z "$bin" ]; then
        [ "$required" = "1" ] && printf '[conductor] accountability binding prerequisites are missing\n' >&2 && return 1
        return 0
    fi
    if [ ! -x "$bin" ] && ! command -v "$bin" >/dev/null 2>&1; then
        [ "$required" = "1" ] && printf '[conductor] accountability binary is unavailable: %s\n' "$bin" >&2 && return 1
        return 0
    fi
    local slug state_root launch
    slug="$(printf '%s' "$repo" | tr '/:' '__')"
    state_root="${AUTOSPEC_AUTONOMOUS_OPERATOR_DIR:-$HOME/.autospec/autonomous-operator}"
    launch="$state_root/$slug/launch.json"
    if [ ! -f "$launch" ] || ! command -v jq >/dev/null 2>&1 \
        || ! jq -e '.accountability.run_id | strings | length > 0' "$launch" >/dev/null 2>&1; then
        [ "$required" = "1" ] && printf '[conductor] verified accountability launch binding is unavailable\n' >&2 && return 1
        return 0
    fi
    local args=(autonomous accountability-event --repo "$repo" --kind "$kind" \
        --what "$what" --why "$why" --evidence "$evidence")
    [ "$project" = "1" ] && args+=(--project)
    "$bin" "${args[@]}" >/dev/null
}

# _autospec_conductor_record_stop: emit one terminal marker and persist terminal
# state.  Uses globals because POSIX signal traps cannot receive Bash locals
# safely while the conductor may be interrupted inside a child command.
_autospec_conductor_record_stop() {
    local reason="${1:-unknown}"
    local cycle="${2:-${_AUTOSPEC_CONDUCTOR_CYCLE:-0}}"
    local shape="${3:-normal}"
    if [ "${_AUTOSPEC_CONDUCTOR_STOP_RECORDED:-0}" = "1" ]; then
        return 0
    fi
    _AUTOSPEC_CONDUCTOR_STOP_RECORDED=1
    local accountability_kind="stopped"
    case "$reason" in
        *park*) accountability_kind="parked" ;;
        all-done|completed) accountability_kind="completed" ;;
    esac
    local accountability_status=0
    _autospec_conductor_accountability_event "$accountability_kind" \
        "Conductor stopped after ${cycle} cycle(s)" \
        "The terminal boundary records why autonomous mutation ended" \
        "$reason" 1 || accountability_status=$?
    if [ "$shape" = "signal" ]; then
        printf '[conductor] stopped: %s (cycle=%s)\n' "$reason" "$cycle" >&2
    else
        printf '[conductor] stopped: %s (cycles=%s)\n' "$reason" "$cycle" >&2
    fi
    if [ -n "${_AUTOSPEC_CONDUCTOR_RESILIENCE:-}" ] \
        && [ -f "$_AUTOSPEC_CONDUCTOR_RESILIENCE" ] \
        && [ -n "${_AUTOSPEC_CONDUCTOR_REPO:-}" ]; then
        bash "$_AUTOSPEC_CONDUCTOR_RESILIENCE" state write \
            --repo "$_AUTOSPEC_CONDUCTOR_REPO" \
            --status "stopped:${reason}:cycle-${cycle}" \
            --session "${_AUTOSPEC_CONDUCTOR_SESSION:-}" \
            2>/dev/null || true
    fi
    if [ "$accountability_status" -ne 0 ]; then
        printf '[conductor] accountability terminal event journal failed; conductor halted\n' >&2
        return "$accountability_status"
    fi
}
