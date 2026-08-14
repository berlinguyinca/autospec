#!/usr/bin/env bash

accountability_state_for_dir() {
    _accountability_dir="$1"
    _accountability_launch="$_accountability_dir/launch.json"
    _accountability_wanted="$(json_from_file "$_accountability_launch" '.accountability.run_id // empty' "")"
    if [ -f "$_accountability_dir/accountability/accountability.json" ]; then
        printf '%s\n' "$_accountability_dir/accountability/accountability.json"
        return 0
    fi
    for _accountability_candidate in "$_accountability_dir"/accountability-resumes/*/accountability.json; do
        [ -f "$_accountability_candidate" ] || continue
        _accountability_found="$(json_from_file "$_accountability_candidate" '.launch.identity.run_id // empty' "")"
        if [ -n "$_accountability_wanted" ] && [ "$_accountability_found" = "$_accountability_wanted" ]; then
            printf '%s\n' "$_accountability_candidate"
            return 0
        fi
    done
    printf '%s\n' "$_accountability_dir/accountability/accountability.json"
}


conductor_row_load() {
    _dir="$1"
    _slug="$(basename "$_dir")"
    _pid="$(tr -d '[:space:]' < "$_dir/conductor.pid")"
    _alive=false
    is_pid_alive "$_pid" && _alive=true
    _log=""
    [ -f "$_dir/conductor.logpath" ] && _log="$(sed -n '1p' "$_dir/conductor.logpath")"
    if [ -z "$_log" ] || [ ! -f "$_log" ]; then
        _log="$(legacy_flat_logpath || true)"
    fi
    _launch="$(launch_file_for_state_dir "$_dir")"
    _repo="$(json_from_file "$_launch" '.repo // empty' "")"
    [ -n "$_repo" ] || _repo="$(slug_to_repo "$_slug")"
    _repo_dir="$(json_from_file "$_launch" '.repo_dir // empty' "")"
    _started_at="$(json_from_file "$_launch" '.started_at // empty' "")"
    _tty="$(json_from_file "$_launch" '.tty // empty' "")"
    _session_id="$(json_from_file "$_launch" '.session_id // empty' "")"
    _argv="$(json_compact_from_file "$_launch" '.argv // []' '[]')"
    _accountability_state="$(accountability_state_for_dir "$_dir")"
    _accountability_run_id="$(json_from_file "$_launch" '.accountability.run_id // empty' "")"
    _accountability_epic="$(json_from_file "$_launch" '.accountability.epic_number // empty' "")"
    _accountability_url="$(json_from_file "$_launch" '.accountability.epic_url // empty' "")"
    _accountability_events="$(json_from_file "$_accountability_state" '.event_count // 0' "0")"
    _accountability_pending="$(json_from_file "$_accountability_state" '.pending_projection_count // 0' "0")"
    _accountability_lifecycle="$(json_from_file "$_accountability_state" 'if .recovery_state == "parked" then "parked" elif .recovery_state == "terminal" then "terminal" else (.lifecycle_phase // empty) end' "")"
    _accountability_recovery="$(json_from_file "$_accountability_state" '.recovery_state // empty' "")"
    _accountability_last_projected="$(json_from_file "$_accountability_state" '.last_projected_at // empty' "")"
    _accountability_next_retry="$(json_from_file "$_accountability_state" '.next_projection_retry_at // empty' "")"
    _accountability_created_at="$(json_from_file "$_accountability_state" '.created_at // empty' "")"
    _accountability_updated_at="$(json_from_file "$_accountability_state" '.updated_at // empty' "")"
    _accountability_projection="current"
    [ -n "$_accountability_run_id" ] || _accountability_projection="unbound"
    [ "${_accountability_pending:-0}" = "0" ] || _accountability_projection="degraded"
    _state_file="$HOME/.autospec/autonomous/$_slug/state.json"
    _state_status="$(json_from_file "$_state_file" '.status // empty' "")"
    _last_cycle="$(json_from_file "$_state_file" '(.cycle // .last_cycle // "")' "")"
    _heartbeat="$(json_from_file "$_state_file" '(.heartbeat_at // .updated_at // "")' "")"
    _heartbeat_age="$(heartbeat_age_seconds "$_heartbeat")"
    _park_state=""
    case "$_state_status" in
        parked*|soft-park*|*park*) _park_state="$_state_status" ;;
    esac
}

print_conductor_json_row() {
    printf '{'
    printf '"slug":%s' "$(json_escape "$_slug")"
    printf ',"repo":%s' "$(json_escape "$_repo")"
    printf ',"pid":%s' "$(json_escape "$_pid")"
    printf ',"alive":%s' "$_alive"
    printf ',"log":%s' "$(json_escape "$_log")"
    printf ',"last_cycle":%s' "$(json_escape "$_last_cycle")"
    printf ',"heartbeat_at":%s' "$(json_escape "$_heartbeat")"
    printf ',"heartbeat_age_seconds":%s' "$(json_escape "$_heartbeat_age")"
    printf ',"park_state":%s' "$(json_escape "$_park_state")"
    printf ',"state_status":%s' "$(json_escape "$_state_status")"
    printf ',"started_at":%s' "$(json_escape "$_started_at")"
    printf ',"tty":%s' "$(json_escape "$_tty")"
    printf ',"session_id":%s' "$(json_escape "$_session_id")"
    printf ',"repo_dir":%s' "$(json_escape "$_repo_dir")"
    printf ',"argv":%s' "$_argv"
    printf ',"accountability":{"run_id":%s,"epic_number":%s,"epic_url":%s,"accountability_state":%s,"recovery_state":%s,"event_count":%s,"pending_projection_count":%s,"last_projected_at":%s,"next_projection_retry_at":%s,"created_at":%s,"updated_at":%s,"projection_state":%s}' \
        "$(json_escape "$_accountability_run_id")" \
        "${_accountability_epic:-null}" \
        "$(json_escape "$_accountability_url")" \
        "$(json_escape "$_accountability_lifecycle")" \
        "$(json_escape "$_accountability_recovery")" \
        "${_accountability_events:-0}" \
        "${_accountability_pending:-0}" \
        "${_accountability_last_projected:-null}" \
        "${_accountability_next_retry:-null}" \
        "${_accountability_created_at:-null}" \
        "${_accountability_updated_at:-null}" \
        "$(json_escape "$_accountability_projection")"
    printf '}'
}

print_conductor_text_row() {
    info "  $_repo ($_slug)"
    info "    pid: ${_pid:-n/a} alive=$_alive"
    info "    log: ${_log:-n/a}"
    [ -n "$_state_status$_last_cycle$_heartbeat_age" ] && \
        info "    state: ${_state_status:-n/a} cycle=${_last_cycle:-n/a} heartbeat_age_seconds=${_heartbeat_age:-n/a}"
    [ -n "$_started_at$_tty$_session_id" ] && \
        info "    launch: started_at=${_started_at:-n/a} tty=${_tty:-n/a} session=${_session_id:-n/a}"
    [ -n "$_accountability_run_id" ] && \
        info "    accountability: $_accountability_projection epic=${_accountability_url:-n/a} events=${_accountability_events:-0}"
}
