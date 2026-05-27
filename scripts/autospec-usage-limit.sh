#!/usr/bin/env bash
# autospec-usage-limit.sh — deterministic usage-limit pause/resume supervisor.
#
# The LLM may be unable to reason after a quota/context limit is reached. This
# helper records the known reset time and exact resume command before exiting,
# then a plain shell daemon polls until the reset time and relaunches the command.

set -euo pipefail

DEFAULT_STATE_DIR="${HOME}/.autospec/usage-limits"
DEFAULT_INTERVAL_SECONDS=300

usage() {
    cat <<'EOF'
Usage:
  autospec-usage-limit.sh arm --harness <claude|codex|opencode> --repo-dir <dir> --command <cmd> (--resume-at <ISO8601>|--wait-seconds <N>) [--run-id <id>] [--interval-seconds <N>] [--state-dir <dir>] [--no-daemon]
  autospec-usage-limit.sh poll --run-id <id> [--state-dir <dir>] [--foreground]
  autospec-usage-limit.sh daemon --run-id <id> [--state-dir <dir>]
  autospec-usage-limit.sh status --run-id <id> [--state-dir <dir>]
  autospec-usage-limit.sh clear --run-id <id> [--state-dir <dir>]

The daemon polls every 300 seconds by default and launches the recorded command
after the resume time. Use --foreground in tests to run the command inline.
EOF
}

die() {
    printf 'autospec-usage-limit: %s\n' "$1" >&2
    exit 1
}

info() {
    printf '[autospec-usage-limit] %s\n' "$*"
}

require_jq() {
    command -v jq >/dev/null 2>&1 || die "jq is required"
}

iso_now() {
    date -u +'%Y-%m-%dT%H:%M:%SZ'
}

epoch_now() {
    date -u +%s
}

iso_to_epoch() {
    ts="$1"
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

epoch_to_iso() {
    epoch="$1"
    date -u -r "$epoch" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "@$epoch" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || die "could not format epoch: $epoch"
}

sanitize_run_id() {
    printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-'
}

state_file_for() {
    state_dir="$1"
    run_id="$2"
    printf '%s/%s.json\n' "$state_dir" "$(sanitize_run_id "$run_id")"
}

write_json_atomic() {
    target="$1"
    mkdir -p "$(dirname "$target")"
    tmp="$(mktemp "${target}.XXXXXX")"
    cat > "$tmp"
    mv "$tmp" "$target"
}

update_state() {
    file="$1"
    shift
    require_jq
    jq "$@" "$file" | write_json_atomic "$file"
}

default_run_id() {
    harness="$1"
    repo_dir="$2"
    repo_slug="$(cd "$repo_dir" 2>/dev/null && git remote get-url origin 2>/dev/null \
        | sed -E 's#^git@[^:]+:##; s#^https?://[^/]+/##; s#\\.git$##; s#[/:]#_#g' \
        || true)"
    [ -n "$repo_slug" ] || repo_slug="$(printf '%s' "$repo_dir" | sed 's#[^A-Za-z0-9._-]#_#g')"
    printf '%s-%s\n' "$harness" "$repo_slug"
}

command_name="${1:-}"
[ -n "$command_name" ] || { usage; exit 1; }
case "$command_name" in --help|-h) usage; exit 0 ;; esac
shift

STATE_DIR="$DEFAULT_STATE_DIR"
RUN_ID=""
HARNESS=""
REPO_DIR=""
RESUME_COMMAND=""
RESUME_AT=""
WAIT_SECONDS=""
INTERVAL_SECONDS="$DEFAULT_INTERVAL_SECONDS"
NO_DAEMON=0
FOREGROUND=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --state-dir) STATE_DIR="${2:-}"; shift 2 ;;
        --run-id) RUN_ID="${2:-}"; shift 2 ;;
        --harness) HARNESS="${2:-}"; shift 2 ;;
        --repo-dir) REPO_DIR="${2:-}"; shift 2 ;;
        --command) RESUME_COMMAND="${2:-}"; shift 2 ;;
        --resume-at) RESUME_AT="${2:-}"; shift 2 ;;
        --wait-seconds) WAIT_SECONDS="${2:-}"; shift 2 ;;
        --interval-seconds) INTERVAL_SECONDS="${2:-}"; shift 2 ;;
        --no-daemon) NO_DAEMON=1; shift ;;
        --foreground) FOREGROUND=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$INTERVAL_SECONDS" in *[!0-9]*|'') die "--interval-seconds must be an integer" ;; esac
[ "$INTERVAL_SECONDS" -gt 0 ] || die "--interval-seconds must be > 0"

case "$command_name" in
    arm)
        require_jq
        [ -n "$HARNESS" ] || die "--harness is required"
        case "$HARNESS" in claude|codex|opencode) ;; *) die "--harness must be claude, codex, or opencode" ;; esac
        [ -n "$REPO_DIR" ] || die "--repo-dir is required"
        [ -d "$REPO_DIR" ] || die "--repo-dir does not exist: $REPO_DIR"
        [ -n "$RESUME_COMMAND" ] || die "--command is required"
        if [ -n "$RESUME_AT" ] && [ -n "$WAIT_SECONDS" ]; then
            die "use either --resume-at or --wait-seconds, not both"
        fi
        if [ -n "$WAIT_SECONDS" ]; then
            case "$WAIT_SECONDS" in *[!0-9]*|'') die "--wait-seconds must be an integer" ;; esac
            RESUME_AT="$(epoch_to_iso "$(( $(epoch_now) + WAIT_SECONDS ))")"
        fi
        [ -n "$RESUME_AT" ] || die "--resume-at or --wait-seconds is required"
        [ "$(iso_to_epoch "$RESUME_AT")" -gt 0 ] || die "--resume-at must be ISO8601 UTC, e.g. 2026-05-27T03:00:00Z"
        [ -n "$RUN_ID" ] || RUN_ID="$(default_run_id "$HARNESS" "$REPO_DIR")"
        state_file="$(state_file_for "$STATE_DIR" "$RUN_ID")"
        log_dir="${HOME}/.autospec/logs"
        mkdir -p "$STATE_DIR" "$log_dir"
        run_log="$log_dir/usage-limit-$(sanitize_run_id "$RUN_ID").log"
        daemon_log="$log_dir/usage-limit-$(sanitize_run_id "$RUN_ID").daemon.log"
        jq -n \
            --argjson schema 1 \
            --arg run_id "$RUN_ID" \
            --arg harness "$HARNESS" \
            --arg repo_dir "$REPO_DIR" \
            --arg command "$RESUME_COMMAND" \
            --arg resume_at "$RESUME_AT" \
            --arg created_at "$(iso_now)" \
            --arg status "waiting" \
            --arg run_log "$run_log" \
            --arg daemon_log "$daemon_log" \
            --arg last_poll_at "" \
            --arg resumed_at "" \
            --argjson interval_seconds "$INTERVAL_SECONDS" \
            --argjson attempts 0 \
            '{schema:$schema,run_id:$run_id,harness:$harness,repo_dir:$repo_dir,command:$command,resume_at:$resume_at,interval_seconds:$interval_seconds,status:$status,attempts:$attempts,created_at:$created_at,last_poll_at:$last_poll_at,resumed_at:$resumed_at,run_log:$run_log,daemon_log:$daemon_log}' \
            | write_json_atomic "$state_file"
        info "armed $RUN_ID; resume_at=$RESUME_AT"
        if [ "$NO_DAEMON" -eq 0 ]; then
            nohup bash "$0" daemon --run-id "$RUN_ID" --state-dir "$STATE_DIR" > "$daemon_log" 2>&1 &
            daemon_pid="$!"
            update_state "$state_file" --argjson pid "$daemon_pid" '.daemon_pid = $pid'
            info "daemon pid=$daemon_pid interval=${INTERVAL_SECONDS}s"
        fi
        ;;
    poll)
        require_jq
        [ -n "$RUN_ID" ] || die "--run-id is required"
        state_file="$(state_file_for "$STATE_DIR" "$RUN_ID")"
        [ -f "$state_file" ] || die "state not found for run-id: $RUN_ID"
        status="$(jq -r '.status // empty' "$state_file")"
        case "$status" in
            succeeded|resumed)
                cat "$state_file"
                exit 0
                ;;
            failed)
                cat "$state_file"
                exit 1
                ;;
            waiting) ;;
            *) die "state is not waiting: ${status:-<empty>}" ;;
        esac
        resume_at="$(jq -r '.resume_at' "$state_file")"
        resume_epoch="$(iso_to_epoch "$resume_at")"
        [ "$resume_epoch" -gt 0 ] || die "invalid resume_at in state: $resume_at"
        now_epoch="$(epoch_now)"
        update_state "$state_file" --arg at "$(iso_now)" '.last_poll_at = $at'
        if [ "$now_epoch" -lt "$resume_epoch" ]; then
            info "waiting until $resume_at"
            exit 2
        fi
        repo_dir="$(jq -r '.repo_dir' "$state_file")"
        resume_command="$(jq -r '.command' "$state_file")"
        run_log="$(jq -r '.run_log' "$state_file")"
        mkdir -p "$(dirname "$run_log")"
        update_state "$state_file" --arg at "$(iso_now)" '.status = "launching" | .attempts += 1 | .resumed_at = $at'
        if [ "$FOREGROUND" -eq 1 ]; then
            set +e
            ( cd "$repo_dir" && bash -lc "$resume_command" )
            rc="$?"
            set -e
            if [ "$rc" -eq 0 ]; then
                update_state "$state_file" '.status = "succeeded"'
            else
                update_state "$state_file" --argjson rc "$rc" '.status = "failed" | .exit_code = $rc'
            fi
            exit "$rc"
        fi
        (
            cd "$repo_dir"
            nohup bash -lc "$resume_command" > "$run_log" 2>&1 &
            printf '%s\n' "$!" > "${run_log}.pid"
        )
        child_pid="$(cat "${run_log}.pid" 2>/dev/null || echo "")"
        if [ -n "$child_pid" ]; then
            update_state "$state_file" --argjson pid "$child_pid" '.status = "resumed" | .child_pid = $pid'
        else
            update_state "$state_file" '.status = "resumed"'
        fi
        info "resumed $RUN_ID pid=${child_pid:-unknown}"
        ;;
    daemon)
        require_jq
        [ -n "$RUN_ID" ] || die "--run-id is required"
        state_file="$(state_file_for "$STATE_DIR" "$RUN_ID")"
        [ -f "$state_file" ] || die "state not found for run-id: $RUN_ID"
        while :; do
            if bash "$0" poll --run-id "$RUN_ID" --state-dir "$STATE_DIR"; then
                exit 0
            fi
            rc="$?"
            [ "$rc" -eq 2 ] || exit "$rc"
            interval="$(jq -r '.interval_seconds // 300' "$state_file" 2>/dev/null || echo 300)"
            case "$interval" in *[!0-9]*|'') interval="$DEFAULT_INTERVAL_SECONDS" ;; esac
            sleep "$interval"
        done
        ;;
    status)
        require_jq
        [ -n "$RUN_ID" ] || die "--run-id is required"
        state_file="$(state_file_for "$STATE_DIR" "$RUN_ID")"
        [ -f "$state_file" ] || die "state not found for run-id: $RUN_ID"
        jq . "$state_file"
        ;;
    clear)
        [ -n "$RUN_ID" ] || die "--run-id is required"
        state_file="$(state_file_for "$STATE_DIR" "$RUN_ID")"
        rm -f "$state_file"
        info "cleared $RUN_ID"
        ;;
    *)
        usage >&2
        exit 1
        ;;
esac
