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
LOG_OVERRIDE=0
MONITOR_INTERVAL=300
MONITOR_ITERATIONS=0
CONDUCTOR_MAX_CYCLES="${CONDUCTOR_MAX_CYCLES:-}"
CONDUCTOR_POLL_INTERVAL="${CONDUCTOR_POLL_INTERVAL:-}"
CONDUCTOR_DRY_RUN="${CONDUCTOR_DRY_RUN:-0}"
CONDUCTOR_NO_DIGEST="${CONDUCTOR_NO_DIGEST:-0}"
AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-}"
AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES="${AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES:-}"

usage() {
    cat <<'EOF'
Usage: autospec-autonomous [start|status|timeline|monitor|logs|watch|stop|restart] [options]

Commands:
  start      Start the detached autonomous conductor (default).
  status     Print PID, log path, conductor state, and spend ledger summary.
  timeline   Print a chronological plain-English activity report.
  monitor    Print the timeline/report repeatedly; default interval is 300 seconds.
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
  --lines N               Log lines for logs/status/timeline output.
  --interval-sec N        Monitor refresh interval. Default 300.
  --iterations N          Monitor iteration cap. Default unlimited.
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

print_timeline() {
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(read_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    if ! command -v python3 >/dev/null 2>&1; then
        die "timeline requires python3"
    fi

    python3 - "$_log" "$LINES" <<'PY'
from collections import deque
from datetime import datetime, timezone
import json
import re
import sys

log_path = sys.argv[1]
try:
    line_count = int(sys.argv[2])
except (IndexError, ValueError):
    line_count = 200

with open(log_path, "r", encoding="utf-8", errors="replace") as handle:
    all_lines = handle.readlines()
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


def issue_timings(lines):
    history = {}
    for obj in json_objects_from_text("\n".join(lines)):
        if not isinstance(obj, dict) or "issue" not in obj or "ts" not in obj:
            continue
        try:
            issue = int(obj["issue"])
            ts = int(obj["ts"])
        except (TypeError, ValueError):
            continue
        step = " ".join(str(obj.get("step") or "working").replace("_", " ").split())
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
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(read_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" "$_log"
}

watch_logs() {
    if [ "$LOG_OVERRIDE" -eq 1 ]; then
        _log="${AUTOSPEC_AUTONOMOUS_LOG:-}"
    else
        _log="$(read_logpath || true)"
    fi
    [ -n "$_log" ] || die "no conductor log path recorded"
    [ -f "$_log" ] || die "conductor log not found: $_log"
    tail -n "$LINES" -f "$_log"
}

stop_conductor() {
    _stop="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/autospec-stop.sh"
    [ -x "$_stop" ] || die "missing stop helper: $_stop"
    bash "$_stop" "$STOP_MODE"
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

while [ $# -gt 0 ]; do
    case "$1" in
        start|status|timeline|monitor|logs|watch|stop|restart|run-foreground)
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
    timeline)
        print_timeline
        ;;
    monitor)
        monitor_report
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
