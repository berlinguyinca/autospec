#!/usr/bin/env bash
# autospec-autonomous.sh — operator lifecycle wrapper for the autonomous conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

STATE_DIR="${AUTOSPEC_AUTONOMOUS_OPERATOR_DIR:-$HOME/.autospec/autonomous-operator}"
PID_FILE="${AUTOSPEC_AUTONOMOUS_PID_FILE:-$STATE_DIR/conductor.pid}"
LOGPATH_FILE="${AUTOSPEC_AUTONOMOUS_LOGPATH_FILE:-$STATE_DIR/conductor.logpath}"
DEFAULT_LOG_DIR="${AUTOSPEC_AUTONOMOUS_LOG_DIR:-$HOME/.autospec/logs}"

ACTION="start"
JSON=0
LINES=80
FORCE=0
STOP_MODE="--graceful"
FOREGROUND=0
CONDUCTOR_MAX_CYCLES="${CONDUCTOR_MAX_CYCLES:-}"
CONDUCTOR_POLL_INTERVAL="${CONDUCTOR_POLL_INTERVAL:-}"
CONDUCTOR_DRY_RUN="${CONDUCTOR_DRY_RUN:-0}"
CONDUCTOR_NO_DIGEST="${CONDUCTOR_NO_DIGEST:-0}"
AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-}"
AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES:-}"

usage() {
    cat <<'EOF'
Usage: autospec-autonomous [start|status|logs|watch|stop|restart] [options]

Commands:
  start      Start the detached autonomous conductor (default).
  status     Print PID, log path, conductor state, and spend ledger summary.
  logs       Print the current conductor log tail.
  watch      Follow the current conductor log.
  stop       Write the autospec stop sentinel for a running conductor.
  restart    Stop if needed, then start a detached conductor.

Options:
  --max-cycles N          Set CONDUCTOR_MAX_CYCLES.
  --dry-run               Run conductor cycles without invoking autospec-run.
  --no-digest             Skip daily digest writes.
  --poll-interval-sec N   Set CONDUCTOR_POLL_INTERVAL.
  --budget-tokens N       Set AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS.
  --budget-issues N       Set AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES.
  --repo-dir DIR          Run autospec-run from this checkout.
  --repo OWNER/REPO       Override GitHub repo slug for conductor helpers.
  --log PATH              Write the conductor log to PATH.
  --lines N               Log lines for logs/status output.
  --json                  Machine-readable status output.
  --foreground            Run in the current shell instead of detaching.
  --force                 Replace stale PID metadata or restart a live process.
  --graceful              Stop after the current iteration (default).
  --immediate             Request immediate stop at the next boundary.
EOF
}

die() {
    printf 'autospec-autonomous: %s\n' "$*" >&2
    exit 1
}

info() {
    printf '%s\n' "$*"
}

json_escape() {
    printf '"'
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
    printf '"'
}

is_pid_alive() {
    _pid="${1:-}"
    [ -n "$_pid" ] || return 1
    kill -0 "$_pid" >/dev/null 2>&1
}

read_pid() {
    if [ -f "$PID_FILE" ]; then
        tr -d '[:space:]' < "$PID_FILE"
    fi
}

read_logpath() {
    if [ -f "$LOGPATH_FILE" ]; then
        sed -n '1p' "$LOGPATH_FILE"
    fi
}

detect_repo_slug() {
    if [ -n "${CONDUCTOR_REPO:-}" ]; then
        printf '%s\n' "$CONDUCTOR_REPO"
        return 0
    fi
    if command -v gh >/dev/null 2>&1; then
        gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null && return 0
    fi
    printf ''
}

current_state_file() {
    _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
    if [ -n "$_repo" ]; then
        _slug="$(printf '%s' "$_repo" | tr '/:' '__')"
        printf '%s/.autospec/autonomous/%s/state.json\n' "$HOME" "$_slug"
        return 0
    fi
    printf ''
}

current_ledger_file() {
    _repo_dir="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    _origin="$(git -C "$_repo_dir" config --get remote.origin.url 2>/dev/null || true)"
    if [ -n "$_origin" ]; then
        _slug="$(printf '%s' "$_origin" | sed 's#.*github.com[:/]##; s#/$##' | tr '/:' '__')"
    else
        _repo="$(detect_repo_slug)"
        _slug="$(printf '%s' "$_repo" | tr '/:' '__')"
    fi
    [ -n "$_slug" ] || return 0
    printf '%s/.autospec/autonomous-spend/%s/spend.json\n' "$HOME" "$_slug"
}

print_status() {
    _pid="$(read_pid || true)"
    _alive=false
    if is_pid_alive "$_pid"; then
        _alive=true
    fi
    _log="$(read_logpath || true)"
    _state="$(current_state_file || true)"
    _ledger="$(current_ledger_file || true)"
    _issues=""
    _tokens=""
    if [ -f "$_ledger" ] && command -v jq >/dev/null 2>&1; then
        _issues="$(jq -r '.issues // empty' "$_ledger" 2>/dev/null || true)"
        _tokens="$(jq -r '.tokens // empty' "$_ledger" 2>/dev/null || true)"
    fi

    if [ "$JSON" -eq 1 ]; then
        printf '{'
        printf '"running":%s' "$_alive"
        printf ',"pid":%s' "$(json_escape "$_pid")"
        printf ',"log":%s' "$(json_escape "$_log")"
        printf ',"state_file":%s' "$(json_escape "$_state")"
        printf ',"ledger_file":%s' "$(json_escape "$_ledger")"
        printf ',"issues":%s' "$(json_escape "$_issues")"
        printf ',"tokens":%s' "$(json_escape "$_tokens")"
        printf '}\n'
        return 0
    fi

    info "autospec-autonomous status"
    info "  running: $_alive"
    info "  pid:     ${_pid:-n/a}"
    info "  log:     ${_log:-n/a}"
    info "  state:   ${_state:-n/a}"
    info "  ledger:  ${_ledger:-n/a}"
    if [ -n "$_issues$_tokens" ]; then
        info "  spend:   issues=${_issues:-n/a} tokens=${_tokens:-n/a}"
    fi
    if [ -n "$_log" ] && [ -f "$_log" ]; then
        info ""
        tail -n "$LINES" "$_log"
    fi
}

ensure_not_running() {
    _pid="$(read_pid || true)"
    if is_pid_alive "$_pid"; then
        if [ "$FORCE" -eq 1 ]; then
            kill "$_pid" >/dev/null 2>&1 || true
            sleep 1
        else
            die "conductor already running with pid $_pid; use status, watch, stop, or --force"
        fi
    fi
}

start_foreground() {
    export AUTOSPEC_REPO_DIR="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    export CONDUCTOR_SCRIPTS_DIR="${CONDUCTOR_SCRIPTS_DIR:-$SCRIPT_DIR}"
    export AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"
    export AUTOSPEC_RUN_CMD="${AUTOSPEC_RUN_CMD:-$SCRIPT_DIR/autospec-autonomous-run-drain.sh}"
    [ -n "$CONDUCTOR_MAX_CYCLES" ] && export CONDUCTOR_MAX_CYCLES
    [ -n "$CONDUCTOR_POLL_INTERVAL" ] && export CONDUCTOR_POLL_INTERVAL
    export CONDUCTOR_DRY_RUN CONDUCTOR_NO_DIGEST
    [ -n "$AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS" ] && export AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS
    [ -n "$AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES" ] && export AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES

    cd "$AUTOSPEC_REPO_DIR"
    . "$SCRIPT_DIR/lib/autospec-loop.sh"
    autospec_conductor_run
}

start_detached() {
    ensure_not_running
    mkdir -p "$STATE_DIR" "$DEFAULT_LOG_DIR"
    _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    if [ -z "$_log" ]; then
        _stamp="$(date -u +%Y%m%dT%H%M%SZ)"
        _log="$DEFAULT_LOG_DIR/autospec-autonomous-$_stamp.log"
    fi
    _repo_dir="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"

    if command -v python3 >/dev/null 2>&1; then
        _pid="$(
            python3 - "$0" "$_log" "$_repo_dir" "$SCRIPT_DIR" "$_repo" <<'PY'
import os, subprocess, sys
script, log_path, repo_dir, scripts_dir, repo = sys.argv[1:6]
env = os.environ.copy()
env["AUTOSPEC_REPO_DIR"] = repo_dir
env["CONDUCTOR_SCRIPTS_DIR"] = scripts_dir
env["AUTOSPEC_SCRIPTS_DIR"] = scripts_dir
if repo:
    env["CONDUCTOR_REPO"] = repo
log = open(log_path, "ab", buffering=0)
p = subprocess.Popen(
    [script, "run-foreground"],
    cwd=repo_dir,
    env=env,
    stdout=log,
    stderr=subprocess.STDOUT,
    start_new_session=True,
)
print(p.pid)
PY
        )"
    else
        AUTOSPEC_REPO_DIR="$_repo_dir" \
        CONDUCTOR_SCRIPTS_DIR="$SCRIPT_DIR" \
        AUTOSPEC_SCRIPTS_DIR="$SCRIPT_DIR" \
        CONDUCTOR_REPO="$_repo" \
            nohup "$0" run-foreground >"$_log" 2>&1 &
        _pid="$!"
    fi

    printf '%s\n' "$_pid" > "$PID_FILE"
    printf '%s\n' "$_log" > "$LOGPATH_FILE"
    info "autospec-autonomous started"
    info "  pid: $_pid"
    info "  log: $_log"
}

show_logs() {
    _log="${AUTOSPEC_AUTONOMOUS_LOG:-$(read_logpath || true)}"
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" "$_log"
}

watch_logs() {
    _log="${AUTOSPEC_AUTONOMOUS_LOG:-$(read_logpath || true)}"
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" -f "$_log"
}

stop_conductor() {
    _stop="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/autospec-stop.sh"
    [ -x "$_stop" ] || die "missing stop helper: $_stop"
    bash "$_stop" "$STOP_MODE"
}

while [ $# -gt 0 ]; do
    case "$1" in
        start|status|logs|watch|stop|restart|run-foreground)
            ACTION="$1"
            ;;
        --max-cycles)
            shift; CONDUCTOR_MAX_CYCLES="${1:-}"
            ;;
        --max-cycles=*)
            CONDUCTOR_MAX_CYCLES="${1#--max-cycles=}"
            ;;
        --dry-run)
            CONDUCTOR_DRY_RUN=1
            ;;
        --no-digest)
            CONDUCTOR_NO_DIGEST=1
            ;;
        --poll-interval-sec)
            shift; CONDUCTOR_POLL_INTERVAL="${1:-}"
            ;;
        --poll-interval-sec=*)
            CONDUCTOR_POLL_INTERVAL="${1#--poll-interval-sec=}"
            ;;
        --budget-tokens)
            shift; AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${1:-}"
            ;;
        --budget-tokens=*)
            AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${1#--budget-tokens=}"
            ;;
        --budget-issues)
            shift; AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${1:-}"
            ;;
        --budget-issues=*)
            AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${1#--budget-issues=}"
            ;;
        --repo-dir)
            shift; AUTOSPEC_REPO_DIR="${1:-}"; export AUTOSPEC_REPO_DIR
            ;;
        --repo-dir=*)
            AUTOSPEC_REPO_DIR="${1#--repo-dir=}"; export AUTOSPEC_REPO_DIR
            ;;
        --repo)
            shift; CONDUCTOR_REPO="${1:-}"; export CONDUCTOR_REPO
            ;;
        --repo=*)
            CONDUCTOR_REPO="${1#--repo=}"; export CONDUCTOR_REPO
            ;;
        --log)
            shift; AUTOSPEC_AUTONOMOUS_LOG="${1:-}"; export AUTOSPEC_AUTONOMOUS_LOG
            ;;
        --log=*)
            AUTOSPEC_AUTONOMOUS_LOG="${1#--log=}"; export AUTOSPEC_AUTONOMOUS_LOG
            ;;
        --lines)
            shift; LINES="${1:-80}"
            ;;
        --lines=*)
            LINES="${1#--lines=}"
            ;;
        --json)
            JSON=1
            ;;
        --foreground)
            FOREGROUND=1
            ;;
        --force)
            FORCE=1
            ;;
        --graceful)
            STOP_MODE="--graceful"
            ;;
        --immediate)
            STOP_MODE="--immediate"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
    shift
done

case "$ACTION" in
    run-foreground)
        start_foreground
        ;;
    start)
        if [ "$FOREGROUND" -eq 1 ]; then
            start_foreground
        else
            start_detached
        fi
        ;;
    status)
        print_status
        ;;
    logs)
        show_logs
        ;;
    watch)
        watch_logs
        ;;
    stop)
        stop_conductor
        ;;
    restart)
        _pid="$(read_pid || true)"
        if is_pid_alive "$_pid"; then
            if [ "$FORCE" -eq 1 ]; then
                kill "$_pid" >/dev/null 2>&1 || true
                sleep 1
            else
                stop_conductor
                _wait=0
                while is_pid_alive "$_pid" && [ "$_wait" -lt 30 ]; do
                    sleep 1
                    _wait=$((_wait + 1))
                done
                if is_pid_alive "$_pid"; then
                    die "stop requested but conductor pid $_pid is still running; rerun restart after it exits or use --force"
                fi
            fi
        fi
        start_detached
        ;;
    *)
        die "unknown action: $ACTION"
        ;;
esac
