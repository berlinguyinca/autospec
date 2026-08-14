#!/usr/bin/env bash
# autospec-autonomous.sh — operator lifecycle wrapper for the autonomous conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib/autospec-status-accountability.sh"
DEFAULT_REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

STATE_ROOT="${AUTOSPEC_AUTONOMOUS_OPERATOR_DIR:-$HOME/.autospec/autonomous-operator}"
STATE_DIR=""
PID_FILE=""
LOGPATH_FILE=""
STOP_FLAG_FILE=""
DEFAULT_LOG_ROOT="${AUTOSPEC_AUTONOMOUS_LOG_DIR:-$HOME/.autospec/logs}"
DEFAULT_LOG_DIR=""
MONITOR_PID_FILE=""
MONITOR_LOGPATH_FILE=""
SUPERVISOR_PID_FILE=""
SUPERVISOR_LOGPATH_FILE=""

ORIGINAL_ARGV=("$@")

ACTION="start"
JSON=0
ALL=0
LINES=80
FORCE=0
STOP_MODE="--graceful"
FOREGROUND=0
LOG_OVERRIDE=0
MONITOR_INTERVAL=300
MONITOR_ITERATIONS=0
AUTOSPEC_AUTONOMOUS_COMPANIONS="${AUTOSPEC_AUTONOMOUS_COMPANIONS:-1}"
AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD="${AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD:-}"
CONDUCTOR_MAX_CYCLES="${CONDUCTOR_MAX_CYCLES:-}"
CONDUCTOR_POLL_INTERVAL="${CONDUCTOR_POLL_INTERVAL:-}"
CONDUCTOR_DRY_RUN="${CONDUCTOR_DRY_RUN:-0}"
CONDUCTOR_NO_DIGEST="${CONDUCTOR_NO_DIGEST:-0}"
AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-}"
AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES:-}"

usage() {
    cat <<'EOF'
Usage: autospec-autonomous [start|list|status|timeline|monitor|supervise|logs|watch|stop|restart] [options]

Commands:
  start      Start the detached autonomous conductor (default).
  list       Enumerate all repo-scoped conductors under the operator directory.
  status     Print PID, log path, conductor state, and spend ledger summary. Use --all for list JSON.
  timeline   Print a chronological plain-English activity report.
  monitor    Print the timeline/report repeatedly; default interval is 300 seconds.
  supervise  Run the deterministic supervisor observer loop; default interval is 300 seconds.
  logs       Print the current conductor log tail.
  watch      Follow the current conductor log.
  stop       Write the autospec stop sentinel for a running conductor.
  restart    Stop if needed, then start a detached conductor.

Options:
  --max-cycles N          Set CONDUCTOR_MAX_CYCLES.
  --dry-run               Run conductor cycles without invoking autospec-run.
  --confirm-preview       Resume from the latest preview and allow implementation.
  --no-digest             Skip daily digest writes.
  --poll-interval-sec N   Set CONDUCTOR_POLL_INTERVAL.
  --budget-tokens N       Set AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS.
  --budget-issues N       Set AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES.
  --repo-dir DIR          Run autospec-run from this checkout. Defaults to the
                          launch cwd's git top-level, else the install dir.
  --repo OWNER/REPO       Override GitHub repo slug for conductor helpers.
  --log PATH              Write the conductor log to PATH.
  --lines N               Log lines for logs/status/timeline output.
  --interval-sec N        Monitor refresh interval. Default 300.
  --iterations N          Monitor iteration cap. Default unlimited.
  --json                  Machine-readable status/list output.
  --all                   With status, enumerate all conductor operator dirs.
  --foreground            Run in the current shell instead of detaching.
  --force                 Replace stale PID metadata or restart a live process.
  AUTOSPEC_AUTONOMOUS_COMPANIONS=0 disables default monitor/supervisor startup.
  AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD overrides the built-in supervisor command.
  AUTOSPEC_PERSONA_SOURCES_CMD overrides intent-source bundle inference.
  AUTOSPEC_BOOTSTRAP_DECISION_CMD handles empty-bundle headless bootstrap filing.
  AUTOSPEC_BOOTSTRAP_INTERVIEW_CMD handles empty-bundle interactive bootstrap.
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

shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}

is_pid_alive() {
    _pid="${1:-}"
    [ -n "$_pid" ] || return 1
    kill -0 "$_pid" >/dev/null 2>&1
}

read_pid() {
    if [ -f "$PID_FILE" ]; then
        tr -d '[:space:]' < "$PID_FILE"
        return 0
    fi
    _legacy="$(legacy_pid_file)"
    if [ -f "$_legacy" ]; then
        tr -d '[:space:]' < "$_legacy"
    fi
}

read_logpath() {
    if [ -f "$LOGPATH_FILE" ]; then
        sed -n '1p' "$LOGPATH_FILE"
        return 0
    fi
    _legacy="$(legacy_logpath_file)"
    if [ -f "$_legacy" ]; then
        sed -n '1p' "$_legacy"
    fi
}

legacy_flat_logpath() {
    [ -d "$DEFAULT_LOG_ROOT" ] || return 0
    find "$DEFAULT_LOG_ROOT" -maxdepth 1 -type f -name 'autospec-autonomous-*.log' 2>/dev/null | sort | tail -n 1
}

resolve_logpath() {
    _recorded="$(read_logpath || true)"
    if [ -n "$_recorded" ] && [ -f "$_recorded" ]; then
        printf '%s
' "$_recorded"
        return 0
    fi
    _legacy_flat="$(legacy_flat_logpath || true)"
    if [ -n "$_legacy_flat" ]; then
        printf '%s
' "$_legacy_flat"
        return 0
    fi
    printf '%s
' "$_recorded"
}

read_scoped_pid() {
    if [ -f "$PID_FILE" ]; then
        tr -d '[:space:]' < "$PID_FILE"
    fi
}

detect_repo_slug() {
    if [ -n "${CONDUCTOR_REPO:-}" ]; then
        printf '%s\n' "$CONDUCTOR_REPO"
        return 0
    fi
    _repo_dir="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    _origin="$(git -C "$_repo_dir" config --get remote.origin.url 2>/dev/null || true)"
    if [ -n "$_origin" ]; then
        printf '%s\n' "$_origin" | sed 's#.*github.com[:/]##; s#/$##; s#\.git$##'
        return 0
    fi
    if command -v gh >/dev/null 2>&1; then
        (cd "$_repo_dir" 2>/dev/null && gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null) && return 0
    fi
    printf ''
}

scope_slug() {
    _repo="$(detect_repo_slug || true)"
    if [ -n "$_repo" ]; then
        printf '%s' "$_repo" | sed 's#[/:]#_#g; s#[^A-Za-z0-9._-]#_#g'
        return 0
    fi
    _repo_dir="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    _real="$(cd "$_repo_dir" 2>/dev/null && pwd -P || printf '%s' "$_repo_dir")"
    printf 'dir_%s' "$(printf '%s' "$_real" | sed 's#[^A-Za-z0-9._-]#_#g')"
}

configure_scope_paths() {
    _scope="$(scope_slug)"
    [ -n "$_scope" ] || _scope="unknown"
    STATE_DIR="${STATE_ROOT%/}/$_scope"
    PID_FILE="${AUTOSPEC_AUTONOMOUS_PID_FILE:-$STATE_DIR/conductor.pid}"
    LOGPATH_FILE="${AUTOSPEC_AUTONOMOUS_LOGPATH_FILE:-$STATE_DIR/conductor.logpath}"
    MONITOR_PID_FILE="${AUTOSPEC_AUTONOMOUS_MONITOR_PID_FILE:-$STATE_DIR/monitor.pid}"
    MONITOR_LOGPATH_FILE="${AUTOSPEC_AUTONOMOUS_MONITOR_LOGPATH_FILE:-$STATE_DIR/monitor.logpath}"
    SUPERVISOR_PID_FILE="${AUTOSPEC_AUTONOMOUS_SUPERVISOR_PID_FILE:-$STATE_DIR/supervisor.pid}"
    SUPERVISOR_LOGPATH_FILE="${AUTOSPEC_AUTONOMOUS_SUPERVISOR_LOGPATH_FILE:-$STATE_DIR/supervisor.logpath}"
    STOP_FLAG_FILE="${AUTOSPEC_STOP_FLAG_FILE:-$STATE_DIR/stop.flag}"
    DEFAULT_LOG_DIR="${DEFAULT_LOG_ROOT%/}/$_scope"
}

legacy_pid_file() {
    printf '%s/conductor.pid\n' "${STATE_ROOT%/}"
}

legacy_logpath_file() {
    printf '%s/conductor.logpath\n' "${STATE_ROOT%/}"
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
    _ledger="$HOME/.autospec/autonomous-spend/$_slug/spend.json"
    if [ -f "$_ledger" ]; then
        printf '%s\n' "$_ledger"
        return 0
    fi
    case "$_slug" in
        *.git) _alt="${_slug%.git}" ;;
        *) _alt="${_slug}.git" ;;
    esac
    _alt_ledger="$HOME/.autospec/autonomous-spend/$_alt/spend.json"
    if [ -f "$_alt_ledger" ]; then
        printf '%s\n' "$_alt_ledger"
        return 0
    fi
    printf '%s\n' "$_ledger"
}


slug_to_repo() {
    _slug="$1"
    case "$_slug" in
        *_*)
            _owner="${_slug%%_*}"
            _name="${_slug#*_}"
            printf '%s/%s\n' "$_owner" "$_name"
            ;;
        *)
            printf '%s\n' "$_slug"
            ;;
    esac
}

launch_file_for_state_dir() {
    printf '%s/launch.json\n' "$1"
}

write_launch_provenance() {
    _repo_dir="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
    _started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    _tty="${TTY:-$(tty 2>/dev/null || true)}"
    _session_id="${STY:-${TMUX_PANE:-}}"
    if [ -z "$_session_id" ]; then
        _session_id="$(ps -o sid= -p $$ 2>/dev/null | tr -d '[:space:]' || true)"
    fi
    mkdir -p "$STATE_DIR"
    {
        printf '{'
        printf '"argv":['
        _arg_first=1
        for _arg in "${ORIGINAL_ARGV[@]}"; do
            if [ "$_arg_first" -eq 0 ]; then printf ','; fi
            _arg_first=0
            json_escape "$_arg"
        done
        printf ']'
        printf ',"started_at":%s' "$(json_escape "$_started_at")"
        printf ',"tty":%s' "$(json_escape "$_tty")"
        printf ',"session_id":%s' "$(json_escape "$_session_id")"
        printf ',"repo":%s' "$(json_escape "$_repo")"
        printf ',"repo_dir":%s' "$(json_escape "$_repo_dir")"
        printf '}\n'
    } > "$STATE_DIR/launch.json"
}

json_from_file() {
    _file="$1"
    _expr="$2"
    _default="$3"
    if [ -f "$_file" ] && command -v jq >/dev/null 2>&1; then
        jq -r "$_expr" "$_file" 2>/dev/null || printf '%s\n' "$_default"
    else
        printf '%s\n' "$_default"
    fi
}

json_compact_from_file() {
    _file="$1"
    _expr="$2"
    _default="$3"
    if [ -f "$_file" ] && command -v jq >/dev/null 2>&1; then
        jq -c "$_expr" "$_file" 2>/dev/null || printf '%s\n' "$_default"
    else
        printf '%s\n' "$_default"
    fi
}

heartbeat_age_seconds() {
    _heartbeat="$1"
    case "$_heartbeat" in
        ''|*[!0-9]*) printf '' ; return 0 ;;
    esac
    _now="$(date +%s)"
    printf '%s\n' "$((_now - _heartbeat))"
}

print_conductor_list() {
    if [ ! -d "$STATE_ROOT" ]; then
        if [ "$JSON" -eq 1 ]; then
            printf '{"conductors":[]}\n'
        else
            info "autospec-autonomous conductors"
            info "  none"
        fi
        return 0
    fi

    _first=1
    [ "$JSON" -eq 1 ] && printf '{"conductors":[' || info "autospec-autonomous conductors"
    for _dir in "$STATE_ROOT"/*; do
        [ -d "$_dir" ] || continue
        [ -f "$_dir/conductor.pid" ] || continue
        conductor_row_load "$_dir"
        if [ "$JSON" -eq 1 ]; then
            [ "$_first" -eq 0 ] && printf ','
            _first=0
            print_conductor_json_row
        else
            print_conductor_text_row
        fi
    done
    [ "$JSON" -eq 1 ] && printf ']}\n'
}

print_status() {
    _pid="$(read_pid || true)"
    _alive=false
    if is_pid_alive "$_pid"; then
        _alive=true
    fi
    _log="$(resolve_logpath || true)"
    _state="$(current_state_file || true)"
    _ledger="$(current_ledger_file || true)"
    _state_status=""
    if [ -f "$_state" ] && command -v jq >/dev/null 2>&1; then
        _state_status="$(jq -r '.status // empty' "$_state" 2>/dev/null || true)"
    fi
    _issues=""
    _tokens=""
    _launch="$STATE_DIR/launch.json"
    _accountability_state="$(accountability_state_for_dir "$STATE_DIR")"
    _accountability_run_id="$(json_from_file "$_launch" '.accountability.run_id // empty' "")"
    _accountability_epic="$(json_from_file "$_launch" '.accountability.epic_number // empty' "")"
    _accountability_url="$(json_from_file "$_launch" '.accountability.epic_url // empty' "")"
    _accountability_events="$(json_from_file "$_accountability_state" '.event_count // 0' "0")"
    _accountability_pending="$(json_from_file "$_accountability_state" '.pending_projection_count // 0' "0")"
    _accountability_lifecycle="$(json_from_file "$_accountability_state" '.lifecycle_phase // empty' "")"
    _accountability_last_projected="$(json_from_file "$_accountability_state" '.last_projected_at // empty' "")"
    _accountability_projection="current"
    [ -n "$_accountability_run_id" ] || _accountability_projection="unbound"
    [ "${_accountability_pending:-0}" = "0" ] || _accountability_projection="degraded"
    if [ -f "$_ledger" ] && command -v jq >/dev/null 2>&1; then
        _issues="$(jq -r '.issues // empty' "$_ledger" 2>/dev/null || true)"
        _tokens="$(jq -r '.tokens // empty' "$_ledger" 2>/dev/null || true)"
    fi

    if [ "$JSON" -eq 1 ]; then
        printf '{'
        printf '"running":%s' "$_alive"
        printf ',"pid":%s' "$(json_escape "$_pid")"
        printf ',"log":%s' "$(json_escape "$_log")"
        printf ',"pid_file":%s' "$(json_escape "$PID_FILE")"
        printf ',"logpath_file":%s' "$(json_escape "$LOGPATH_FILE")"
        printf ',"stop_flag_file":%s' "$(json_escape "$STOP_FLAG_FILE")"
        printf ',"state_file":%s' "$(json_escape "$_state")"
        printf ',"state_status":%s' "$(json_escape "$_state_status")"
        printf ',"ledger_file":%s' "$(json_escape "$_ledger")"
        printf ',"issues":%s' "$(json_escape "$_issues")"
        printf ',"tokens":%s' "$(json_escape "$_tokens")"
        printf ',"accountability":{"run_id":%s,"epic_number":%s,"epic_url":%s,"accountability_state":%s,"event_count":%s,"pending_projection_count":%s,"last_projected_at":%s,"projection_state":%s}' \
            "$(json_escape "$_accountability_run_id")" "${_accountability_epic:-null}" \
            "$(json_escape "$_accountability_url")" "$(json_escape "$_accountability_lifecycle")" \
            "${_accountability_events:-0}" "${_accountability_pending:-0}" \
            "${_accountability_last_projected:-null}" "$(json_escape "$_accountability_projection")"
        printf '}\n'
        return 0
    fi

    info "autospec-autonomous status"
    info "  running: $_alive"
    info "  pid:     ${_pid:-n/a}"
    info "  log:     ${_log:-n/a}"
    info "  pidfile: $PID_FILE"
    info "  stop:    $STOP_FLAG_FILE"
    info "  state:   ${_state:-n/a}"
    if [ -n "$_state_status" ]; then
        info "  status:  $_state_status"
    fi
    info "  ledger:  ${_ledger:-n/a}"
    info "  accountability: $_accountability_projection epic=${_accountability_url:-n/a} events=${_accountability_events:-0}"
    if [ -n "$_issues$_tokens" ]; then
        info "  spend:   issues=${_issues:-n/a} tokens=${_tokens:-n/a}"
    fi
    if [ -n "$_log" ] && [ -f "$_log" ]; then
        info ""
        tail -n "$LINES" "$_log"
    fi
}

print_timeline() {
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(resolve_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    if ! command -v python3 >/dev/null 2>&1; then
        die "timeline requires python3"
    fi
    _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
    _heartbeat_dirs=""
    if [ -n "$_repo" ]; then
        _slug_underscore="$(printf '%s' "$_repo" | tr '/:' '__')"
        _slug_double_underscore="$(printf '%s' "$_repo" | sed 's#[/:]#__#g')"
        _slug_dash="$(printf '%s' "$_repo" | tr '/:' '-')"
        _heartbeat_dirs="$HOME/.autospec/process-heartbeats/$_slug_underscore:$HOME/.autospec/process-heartbeats/$_slug_double_underscore:$HOME/.autospec/process-heartbeats/$_slug_dash"
    fi

    python3 - "$_log" "$LINES" "$_heartbeat_dirs" <<'PY'
from collections import deque
from datetime import datetime, timezone
import json
import os
import re
import sys

log_path = sys.argv[1]
try:
    line_count = int(sys.argv[2])
except (IndexError, ValueError):
    line_count = 200
heartbeat_dirs = []
if len(sys.argv) > 3 and sys.argv[3]:
    heartbeat_dirs = [path for path in sys.argv[3].split(":") if path]

with open(log_path, "r", encoding="utf-8", errors="replace") as handle:
    all_lines = handle.readlines()

for heartbeat_dir in heartbeat_dirs:
    if not os.path.isdir(heartbeat_dir):
        continue
    for name in sorted(os.listdir(heartbeat_dir)):
        if not name.endswith(".json"):
            continue
        path = os.path.join(heartbeat_dir, name)
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as heartbeat:
                all_lines.append(heartbeat.read())
                all_lines.append("\n")
        except OSError:
            continue

lines = list(deque(all_lines, maxlen=max(line_count, 1)))

events = []
current_time = None
last_workdir = ""


def parse_iso(value):
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def set_updated_time(line):
    global current_time
    match = re.search(r'"updated_at"\s*:\s*"([^"]+)"', line)
    if match:
        parsed = parse_iso(match.group(1))
        if parsed is not None:
            current_time = parsed


def set_heartbeat_time(line):
    global current_time
    match = re.match(r"HEARTBEAT_AT:(\d+)\s*$", line)
    if match:
        current_time = datetime.fromtimestamp(int(match.group(1)), tz=timezone.utc)


def add(text, when=None):
    clean = " ".join(text.strip().split())
    if clean:
        events.append((when if when is not None else current_time, clean))


def summarize_paths(paths):
    if not paths:
        return ""
    if len(paths) == 1:
        return paths[0]
    if len(paths) == 2:
        return f"{paths[0]} and {paths[1]}"
    return f"{', '.join(paths[:-1])}, and {paths[-1]}"


def json_objects_from_text(text):
    decoder = json.JSONDecoder()
    index = 0
    while index < len(text):
        start = text.find("{", index)
        if start == -1:
            break
        try:
            obj, end = decoder.raw_decode(text[start:])
        except json.JSONDecodeError:
            index = start + 1
            continue
        yield obj
        index = start + max(end, 1)


def issue_number(issue):
    if not isinstance(issue, dict):
        return None
    value = issue.get("number")
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return None


def issue_label(issue):
    if not isinstance(issue, dict):
        return ""
    number = issue_number(issue)
    title = " ".join(str(issue.get("title") or "").split())
    if number is not None and title:
        return f"#{number} {title}"
    if number is not None:
        return f"#{number}"
    return title


def unique_issues(*groups):
    by_number = {}
    unnumbered = []
    for group in groups:
        if not isinstance(group, list):
            continue
        for issue in group:
            label = issue_label(issue)
            if not label:
                continue
            number = issue_number(issue)
            if number is None:
                if label not in unnumbered:
                    unnumbered.append(label)
            else:
                by_number[number] = issue
    return list(by_number.values()) + [{"title": label} for label in unnumbered]


def format_duration(minutes):
    if minutes < 60:
        return f"{minutes} minutes"
    hours = minutes / 60
    if hours.is_integer():
        return f"{int(hours)} hours"
    return f"{hours:.1f} hours"


def format_duration_range(low_minutes, high_minutes):
    if high_minutes < 60:
        return f"{low_minutes}-{high_minutes} minutes"
    if low_minutes >= 60 and low_minutes % 60 == 0 and high_minutes % 60 == 0:
        return f"{low_minutes // 60}-{high_minutes // 60} hours"
    if low_minutes >= 60 and high_minutes >= 60:
        low_hours = low_minutes / 60
        high_hours = high_minutes / 60
        low_text = str(int(low_hours)) if low_hours.is_integer() else f"{low_hours:.1f}"
        high_text = str(int(high_hours)) if high_hours.is_integer() else f"{high_hours:.1f}"
        return f"{low_text}-{high_text} hours"
    return f"{format_duration(low_minutes)}-{format_duration(high_minutes)}"


def format_elapsed(seconds):
    minutes = max(0, int(round(seconds / 60)))
    return format_duration(minutes)


def heartbeat_step_and_ts(obj):
    step = " ".join(str(obj.get("step") or obj.get("status") or "working").replace("_", " ").split())
    ts_value = obj.get("ts")
    if isinstance(ts_value, int):
        return step, ts_value
    if isinstance(ts_value, str) and ts_value.isdigit():
        return step, int(ts_value)
    updated_at = obj.get("updated_at")
    if isinstance(updated_at, str):
        parsed = parse_iso(updated_at)
        if parsed is not None:
            return step, int(parsed.timestamp())
    return step, None


def issue_timings(lines):
    history = {}
    for obj in json_objects_from_text("\n".join(lines)):
        if not isinstance(obj, dict) or "issue" not in obj or "ts" not in obj:
            if not isinstance(obj, dict) or "issue" not in obj or "updated_at" not in obj:
                continue
        if not isinstance(obj, dict):
            continue
        try:
            issue = int(obj["issue"])
        except (TypeError, ValueError):
            continue
        step, ts = heartbeat_step_and_ts(obj)
        if ts is None:
            continue
        record = history.setdefault(issue, {"first": ts, "last": ts, "step": step, "done": False})
        record["first"] = min(record["first"], ts)
        if ts >= record["last"]:
            record["last"] = ts
            record["step"] = step
        if step in {"merged", "complete", "completed", "done", "pr merged"}:
            record["done"] = True
            record["last"] = max(record["last"], ts)
    if not history:
        return []

    active = []
    completed = []
    for issue, record in history.items():
        elapsed = format_elapsed(record["last"] - record["first"])
        if record["done"]:
            completed.append((record["last"], f"#{issue} completed in {elapsed}"))
        else:
            active.append((record["last"], f"#{issue} current step {record['step']} after {elapsed}"))
    active.sort(reverse=True)
    completed.sort(reverse=True)
    rows = ["item timing"]
    rows.extend(text for _, text in active[:3])
    rows.extend(text for _, text in completed[:3])
    return rows


def active_heartbeat_numbers(lines):
    history = {}
    for obj in json_objects_from_text("\n".join(lines)):
        if not isinstance(obj, dict) or "issue" not in obj:
            continue
        try:
            issue = int(obj["issue"])
        except (TypeError, ValueError):
            continue
        step, ts = heartbeat_step_and_ts(obj)
        if ts is None:
            continue
        record = history.setdefault(issue, {"last": ts, "step": step, "done": False})
        if ts >= record["last"]:
            record["last"] = ts
            record["step"] = step
        if step in {"merged", "complete", "completed", "done", "pr merged"}:
            record["done"] = True
            record["last"] = max(record["last"], ts)
    return {issue for issue, record in history.items() if not record["done"]}


def remove_issue_numbers(group, numbers):
    if not numbers:
        return group
    return [issue for issue in group if issue_number(issue) not in numbers]


def latest_forecast(lines):
    text = "\n".join(lines)
    latest = None
    for obj in json_objects_from_text(text):
        if not isinstance(obj, dict):
            continue
        if any(isinstance(obj.get(key), list) for key in ("ready", "claimed", "blocked", "batch")):
            latest = obj
    if latest is None:
        return None

    ready = latest.get("ready") if isinstance(latest.get("ready"), list) else []
    claimed = latest.get("claimed") if isinstance(latest.get("claimed"), list) else []
    blocked = latest.get("blocked") if isinstance(latest.get("blocked"), list) else []
    batch = latest.get("batch") if isinstance(latest.get("batch"), list) else []
    active_numbers = active_heartbeat_numbers(lines)
    candidates_by_number = {
        issue_number(issue): issue
        for issue in unique_issues(ready, claimed, blocked, batch)
        if issue_number(issue) is not None
    }
    promoted_numbers = active_numbers.intersection(candidates_by_number)
    if promoted_numbers:
        already_claimed = {issue_number(issue) for issue in claimed}
        claimed = claimed + [
            candidates_by_number[number]
            for number in sorted(promoted_numbers)
            if number not in already_claimed
        ]
        ready = remove_issue_numbers(ready, promoted_numbers)
        batch = remove_issue_numbers(batch, promoted_numbers)
    all_issues = unique_issues(ready, claimed, blocked, batch)
    total = len(all_issues)
    if total == 0:
        return None

    low_minutes = total * 45
    high_minutes = total * 90
    rows = [
        "autospec-autonomous forecast",
        f"things left: {total} total ({len(ready)} ready, {len(claimed)} in progress, {len(blocked)} blocked)",
        f"rough ETA: about {format_duration_range(low_minutes, high_minutes)} at 45-90 minutes per item",
    ]

    planned_label = ""
    if claimed:
        planned_label = issue_label(claimed[0])
        rows.append(f"planned next: finish {planned_label}")
        rows.append("next item start estimate: after current item finishes, roughly 15-45 minutes of handoff overhead")
    elif batch:
        planned_label = issue_label(batch[0])
        rows.append(f"planned next: start {planned_label}")
        rows.append("next item start estimate: likely within the next conductor cycle")
    elif ready:
        planned_label = issue_label(ready[0])
        rows.append(f"planned next: start {planned_label}")
        rows.append("next item start estimate: likely within the next conductor cycle")

    if batch:
        batch_label = issue_label(batch[0])
        if batch_label and batch_label != planned_label and not any(batch_label == issue_label(issue) for issue in claimed):
            rows.append(f"then start {batch_label}")
    elif claimed and ready:
        rows.append(f"then start {issue_label(ready[0])}")
    elif len(ready) > 1:
        rows.append(f"then start {issue_label(ready[1])}")

    if blocked:
        rows.append(f"blocked later: {issue_label(blocked[0])}")
    return rows


i = 0
while i < len(lines):
    raw = lines[i].rstrip("\n")
    line = raw.strip()
    set_updated_time(line)
    set_heartbeat_time(line)

    if line.startswith("workdir:"):
        last_workdir = line.split(":", 1)[1].strip()

    cycle = re.match(r"\[conductor\] cycle ([0-9]+) starting", line)
    if cycle:
        add(f"started autonomous cycle {cycle.group(1)}")
        i += 1
        continue

    tier = re.match(r"\[conductor\] tier=([0-9]+) action=([a-z0-9_-]+)", line)
    if tier:
        action = tier.group(2).replace("-", " ")
        if tier.group(1) == "1" and tier.group(2) == "run-backlog":
            add("started Tier 1 backlog-to-main work")
        else:
            add(f"started Tier {tier.group(1)} {action}")
        i += 1
        continue

    if "main-health pending" in line and "skipping drain" in line:
        add("skipped the backlog drain because main health was still pending")
        i += 1
        continue

    if "Hook audit addressed." in line:
        add("addressed a hook audit finding")
        i += 1
        continue

    if line == "Changed:":
        changed = []
        j = i + 1
        while j < len(lines):
            item = lines[j].strip()
            if item.startswith("- "):
                changed.append(item[2:].strip("` "))
                j += 1
                continue
            break
        summary = summarize_paths(changed)
        if summary:
            add(f"updated {summary}")
        i = j
        continue

    if line == "Verified:":
        verified = []
        j = i + 1
        while j < len(lines):
            item = lines[j].strip()
            if item.startswith("- "):
                verified.append(item[2:].strip())
                j += 1
                continue
            break
        if verified:
            add(f"verified {len(verified)} checks: {', '.join(verified)}")
        i = j
        continue

    if line == "user" and i + 1 < len(lines) and lines[i + 1].strip() == "$autospec-run":
        if last_workdir:
            add(f"started autospec-run in {last_workdir}")
        else:
            add("started autospec-run")
        i += 2
        continue

    if line.startswith("$autospec-run"):
        add("started autospec-run")
        i += 1
        continue

    pr = re.search(r"https://github\.com/[^/]+/[^/]+/pull/([0-9]+)", line)
    if pr:
        add(f"opened or referenced PR #{pr.group(1)}")

    i += 1


def fmt(when):
    if when is None:
        return "time unknown"
    local = when.astimezone()
    text = local.strftime("%I:%M %p").lstrip("0").lower()
    return text


forecast_rows = latest_forecast(all_lines)
timing_rows = issue_timings(all_lines)

if not events and not forecast_rows and not timing_rows:
    print("No timeline events found in the selected log window.")
    sys.exit(0)

first_known = next((when for when, _ in events if when is not None), None)
if first_known is not None:
    events = [(first_known if when is None else when, text) for when, text in events]

print("autospec-autonomous timeline")
previous = None
seen = set()
for when, text in events:
    row = (fmt(when), text)
    if row == previous or row in seen:
        continue
    previous = row
    seen.add(row)
    suffix = "" if row[1].endswith((".", "!", "?")) else "."
    print(f"{row[0]} - {row[1]}{suffix}")

if forecast_rows:
    if events:
        print("")
    for row in forecast_rows:
        print(row)

if timing_rows:
    if events or forecast_rows:
        print("")
    for row in timing_rows:
        print(row)
PY
}

ensure_not_running() {
    _pid="$(read_scoped_pid || true)"
    if is_pid_alive "$_pid"; then
        if [ "$FORCE" -eq 1 ]; then
            kill "$_pid" >/dev/null 2>&1 || true
            sleep 1
        else
            die "conductor already running with pid $_pid; use status, watch, stop, or --force"
        fi
    fi
}

spawn_background_command() {
    _label="$1"
    _command="$2"
    _log="$3"
    _repo_dir="$4"

    if command -v python3 >/dev/null 2>&1; then
        python3 - "$_label" "$_log" "$_repo_dir" "$_command" <<'PY'
import os, subprocess, sys
label, log_path, repo_dir, command = sys.argv[1:5]
env = os.environ.copy()
env["AUTOSPEC_REPO_DIR"] = repo_dir
log = open(log_path, "ab", buffering=0)
p = subprocess.Popen(
    command,
    cwd=repo_dir,
    env=env,
    stdout=log,
    stderr=subprocess.STDOUT,
    start_new_session=True,
    shell=True,
)
print(p.pid)
PY
    else
        AUTOSPEC_REPO_DIR="$_repo_dir" \
            nohup sh -c "$_command" >"$_log" 2>&1 &
        printf '%s\n' "$!"
    fi
}

start_companion_process() {
    _label="$1"
    _command="$2"
    _pid_file="$3"
    _logpath_file="$4"
    _log="$5"
    _repo_dir="$6"

    _existing_pid=""
    [ -f "$_pid_file" ] && _existing_pid="$(tr -d '[:space:]' < "$_pid_file" || true)"
    if is_pid_alive "$_existing_pid"; then
        info "  $_label pid: $_existing_pid (already running)"
        return 0
    fi

    _pid="$(spawn_background_command "$_label" "$_command" "$_log" "$_repo_dir" 2>>"$_log" || true)"
    if [ -z "$_pid" ]; then
        printf 'autospec-autonomous: warning: failed to start %s companion\n' "$_label" >&2
        return 0
    fi
    printf '%s\n' "$_pid" > "$_pid_file"
    printf '%s\n' "$_log" > "$_logpath_file"
    info "  $_label pid: $_pid"
    info "  $_label log: $_log"
}

start_default_companions() {
    [ "$AUTOSPEC_AUTONOMOUS_COMPANIONS" = "0" ] && return 0

    _repo_dir="$1"
    _repo="$2"
    _interval="${AUTOSPEC_AUTONOMOUS_MONITOR_INTERVAL:-300}"
    _monitor_log="$DEFAULT_LOG_DIR/autospec-autonomous-monitor.log"
    _monitor_cmd="$(shell_quote "$0") monitor --repo-dir $(shell_quote "$_repo_dir") --interval-sec $(shell_quote "$_interval")"
    if [ -n "$_repo" ]; then
        _monitor_cmd="$_monitor_cmd --repo $(shell_quote "$_repo")"
    fi
    start_companion_process "monitor" "$_monitor_cmd" "$MONITOR_PID_FILE" "$MONITOR_LOGPATH_FILE" "$_monitor_log" "$_repo_dir"

    _supervisor_log="$DEFAULT_LOG_DIR/autospec-autonomous-supervisor.log"
    _supervisor_cmd="${AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD:-}"
    if [ -z "$_supervisor_cmd" ]; then
        _supervisor_cmd="$(shell_quote "$0") supervise --repo-dir $(shell_quote "$_repo_dir") --interval-sec $(shell_quote "$_interval")"
        if [ -n "$_repo" ]; then
            _supervisor_cmd="$_supervisor_cmd --repo $(shell_quote "$_repo")"
        fi
    fi
    start_companion_process "supervisor" "$_supervisor_cmd" "$SUPERVISOR_PID_FILE" "$SUPERVISOR_LOGPATH_FILE" "$_supervisor_log" "$_repo_dir"
}

start_foreground() {
    export AUTOSPEC_REPO_DIR="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
    export CONDUCTOR_SCRIPTS_DIR="${CONDUCTOR_SCRIPTS_DIR:-$SCRIPT_DIR}"
    export AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"
    _drain_repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
    _typed_drain_cmd="$(shell_quote "${AUTOSPEC_BIN:-autospec}") autonomous drain --repo $(shell_quote "$_drain_repo") --repo-dir $(shell_quote "$AUTOSPEC_REPO_DIR")"
    export AUTOSPEC_RUN_CMD="${AUTOSPEC_RUN_CMD:-$_typed_drain_cmd}"
    export AUTOSPEC_PERSONA_SOURCES_CMD="${AUTOSPEC_PERSONA_SOURCES_CMD:-$SCRIPT_DIR/autonomous-persona-sources.sh}"
    # Tier-2/3/4 discovery must run the explore SKILL through the LLM harness
    # (mirroring AUTOSPEC_RUN_CMD's drain wrapper). Without this, the loop falls
    # back to bare `bash autospec-explore.sh --once`, which has no orchestrator
    # to dispatch researcher subagents + fail-closed verify — every proposal is
    # refused and discovery is structurally dry. Same `:-` guard so operators/
    # tests can override.
    export AUTOSPEC_EXPLORE_CMD="${AUTOSPEC_EXPLORE_CMD:-$SCRIPT_DIR/autospec-autonomous-explore-drain.sh}"
    # explore's adversarial verify stage is fail-closed: an autonomous --once run
    # with NO skeptic verdicts files ZERO proposals. Without a verifier wired,
    # detached discovery generates proposals every idle cycle but files nothing.
    # This bridge runs the skeptic through the LLM harness (omx). Same `:-` guard
    # so operators/tests can override.
    export AUTOSPEC_EXPLORE_VERIFY_CMD="${AUTOSPEC_EXPLORE_VERIFY_CMD:-bash $SCRIPT_DIR/autospec-autonomous-verify-drain.sh}"
    export AUTOSPEC_STOP_FLAG_FILE="$STOP_FLAG_FILE"
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
    # Resolve + validate the repo dir BEFORE provenance so launch.json, the
    # detect_repo_slug below, and the spawned child all agree on one value.
    # When AUTOSPEC_REPO_DIR is unset (no --repo-dir), prefer the launch cwd's
    # git checkout over DEFAULT_REPO_DIR (=$SCRIPT_DIR/.. = ~/.autospec when
    # installed) so `start --repo OWNER/REPO` targets the real repo, not the
    # installed script copies. DEFAULT_REPO_DIR stays the last-resort fallback.
    _repo_dir="${AUTOSPEC_REPO_DIR:-}"
    if [ -z "$_repo_dir" ]; then
        _repo_dir="$(git rev-parse --show-toplevel 2>/dev/null || true)"
        [ -n "$_repo_dir" ] || _repo_dir="$DEFAULT_REPO_DIR"
    fi
    # Fail loud (Rule 12): never launch the conductor against a non-checkout —
    # the omx-bridged explore/drain would `--cd` into it and analyze garbage,
    # burning tokens silently.
    if ! git -C "$_repo_dir" rev-parse --show-toplevel >/dev/null 2>&1; then
        die "resolved repo dir ($_repo_dir) is not a git checkout — pass --repo-dir /path/to/checkout (or run from inside the checkout)"
    fi
    export AUTOSPEC_REPO_DIR="$_repo_dir"
    # Non-fatal wrong-dir signal: warn when an explicit --repo slug is not present
    # in the checkout's origin remote (forks/mirrors are legitimate — do not die).
    if [ -n "${CONDUCTOR_REPO:-}" ]; then
        _origin_url="$(git -C "$_repo_dir" config --get remote.origin.url 2>/dev/null || true)"
        case "$_origin_url" in
            *"$CONDUCTOR_REPO"*) : ;;
            *) printf 'autospec-autonomous: warning: repo dir %s origin (%s) does not contain --repo %s — possible wrong checkout\n' "$_repo_dir" "$_origin_url" "$CONDUCTOR_REPO" >&2 ;;
        esac
    fi
    mkdir -p "$STATE_DIR" "$DEFAULT_LOG_DIR"
    write_launch_provenance
    _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    if [ -z "$_log" ]; then
        _stamp="$(date -u +%Y%m%dT%H%M%SZ)"
        _log="$DEFAULT_LOG_DIR/autospec-autonomous-$_stamp.log"
    fi
    _repo_dir="$AUTOSPEC_REPO_DIR"
    _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
    export AUTOSPEC_STOP_FLAG_FILE="$STOP_FLAG_FILE"

    if command -v python3 >/dev/null 2>&1; then
        _pid="$(
            python3 - "$0" "$_log" "$_repo_dir" "$SCRIPT_DIR" "$_repo" <<'PY'
import os, subprocess, sys
script, log_path, repo_dir, scripts_dir, repo = sys.argv[1:6]
env = os.environ.copy()
env["AUTOSPEC_REPO_DIR"] = repo_dir
env["CONDUCTOR_SCRIPTS_DIR"] = scripts_dir
env["AUTOSPEC_SCRIPTS_DIR"] = scripts_dir
env["AUTOSPEC_STOP_FLAG_FILE"] = os.environ.get("AUTOSPEC_STOP_FLAG_FILE", "")
if repo:
    env["CONDUCTOR_REPO"] = repo
log = open(log_path, "ab", buffering=0)
p = subprocess.Popen(
    ["bash", script, "run-foreground"],
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
    start_default_companions "$_repo_dir" "$_repo"
}

show_logs() {
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(resolve_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" "$_log"
}

watch_logs() {
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(resolve_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" -f "$_log"
}

stop_conductor() {
    _stop="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/autospec-stop.sh"
    [ -x "$_stop" ] || die "missing stop helper: $_stop"
    AUTOSPEC_STOP_FLAG_FILE="$STOP_FLAG_FILE" bash "$_stop" "$STOP_MODE"
}

kill_conductor_process_group() {
    _group_pid="$1"
    [ -n "$_group_pid" ] || return 0
    # Detached conductors are started with start_new_session=True, so the
    # negative PID targets the entire lifecycle (drains, harnesses, explorers).
    kill -- "-$_group_pid" >/dev/null 2>&1 || kill "$_group_pid" >/dev/null 2>&1 || true
}

monitor_report() {
    _iteration=0
    while :; do
        _iteration=$((_iteration + 1))
        printf 'autospec-autonomous monitor %s\n' "$(date '+%Y-%m-%d %H:%M:%S %Z')"
        print_timeline
        if [ "$MONITOR_ITERATIONS" -gt 0 ] && [ "$_iteration" -ge "$MONITOR_ITERATIONS" ]; then
            break
        fi
        sleep "$MONITOR_INTERVAL"
        printf '\n'
    done
}

supervise_report() {
    _iteration=0
    while :; do
        _iteration=$((_iteration + 1))
        _repo="${CONDUCTOR_REPO:-$(detect_repo_slug)}"
        _pid="$(read_scoped_pid || true)"
        _state="stopped"
        if is_pid_alive "$_pid"; then
            _state="running"
        elif [ ! -f "$STOP_FLAG_FILE" ]; then
            # A supervisor is a liveness companion, not only a reporter: a
            # terminated foreground conductor must be relaunched so a stale
            # worker or transient harness failure cannot leave autonomy off.
            start_detached
            _state="restarted"
            printf 'autospec-supervise: restarted stopped conductor repo=%s\n' "${_repo:-unknown}"
        fi
        printf 'autospec-supervise: ok repo=%s conductor=%s pid=%s action=none\n' "${_repo:-unknown}" "$_state" "${_pid:-}"
        if [ "$MONITOR_ITERATIONS" -gt 0 ] && [ "$_iteration" -ge "$MONITOR_ITERATIONS" ]; then
            break
        fi
        sleep "$MONITOR_INTERVAL"
    done
}

while [ $# -gt 0 ]; do
    case "$1" in
        start|list|status|timeline|monitor|supervise|logs|watch|stop|restart|run-foreground)
            ACTION="$1"
            ;;
        --max-cycles)
            shift; CONDUCTOR_MAX_CYCLES="${1:-}"; export CONDUCTOR_MAX_CYCLES
            ;;
        --max-cycles=*)
            CONDUCTOR_MAX_CYCLES="${1#--max-cycles=}"; export CONDUCTOR_MAX_CYCLES
            ;;
        --dry-run)
            CONDUCTOR_DRY_RUN=1; export CONDUCTOR_DRY_RUN
            ;;
        --confirm-preview)
            CONDUCTOR_DRY_RUN=0; export CONDUCTOR_DRY_RUN
            ;;
        --no-digest)
            CONDUCTOR_NO_DIGEST=1; export CONDUCTOR_NO_DIGEST
            ;;
        --poll-interval-sec)
            shift; CONDUCTOR_POLL_INTERVAL="${1:-}"; export CONDUCTOR_POLL_INTERVAL
            ;;
        --poll-interval-sec=*)
            CONDUCTOR_POLL_INTERVAL="${1#--poll-interval-sec=}"; export CONDUCTOR_POLL_INTERVAL
            ;;
        --budget-tokens)
            shift; AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${1:-}"; export AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS
            ;;
        --budget-tokens=*)
            AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${1#--budget-tokens=}"; export AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS
            ;;
        --budget-issues)
            shift; AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${1:-}"; export AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES
            ;;
        --budget-issues=*)
            AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${1#--budget-issues=}"; export AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES
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
            shift; AUTOSPEC_AUTONOMOUS_LOG="${1:-}"; export AUTOSPEC_AUTONOMOUS_LOG; LOG_OVERRIDE=1
            ;;
        --log=*)
            AUTOSPEC_AUTONOMOUS_LOG="${1#--log=}"; export AUTOSPEC_AUTONOMOUS_LOG; LOG_OVERRIDE=1
            ;;
        --lines)
            shift; LINES="${1:-80}"
            ;;
        --lines=*)
            LINES="${1#--lines=}"
            ;;
        --interval-sec)
            shift; MONITOR_INTERVAL="${1:-300}"
            ;;
        --interval-sec=*)
            MONITOR_INTERVAL="${1#--interval-sec=}"
            ;;
        --iterations)
            shift; MONITOR_ITERATIONS="${1:-0}"
            ;;
        --iterations=*)
            MONITOR_ITERATIONS="${1#--iterations=}"
            ;;
        --json)
            JSON=1
            ;;
        --all)
            ALL=1
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

configure_scope_paths

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
    list)
        print_conductor_list
        ;;
    status)
        if [ "$ALL" -eq 1 ]; then
            print_conductor_list
        else
            print_status
        fi
        ;;
    timeline)
        print_timeline
        ;;
    monitor)
        monitor_report
        ;;
    supervise)
        supervise_report
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
        _pid="$(read_scoped_pid || true)"
        if is_pid_alive "$_pid"; then
            if [ "$FORCE" -eq 1 ]; then
                kill_conductor_process_group "$_pid"
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
