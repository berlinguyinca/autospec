#!/usr/bin/env bash
# autospec-autonomous-run-drain.sh — one Tier-1 drain invocation for the conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

DEFAULT_REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if command -v autospec_runtime_config_path >/dev/null 2>&1; then
    REPO_DIR="$(autospec_runtime_config_path autonomous.repo_dir AUTOSPEC_REPO_DIR "$DEFAULT_REPO_DIR")"
else
    REPO_DIR="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
fi
if command -v autospec_runtime_config_int >/dev/null 2>&1; then
    DRAIN_STALL_SECS="$(autospec_runtime_config_int autonomous.drain.stall_secs AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS 1800)"
    DRAIN_POLL_SECS="$(autospec_runtime_config_int autonomous.drain.poll_secs AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS 15)"
else
    DRAIN_STALL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS:-1800}"
    DRAIN_POLL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS:-15}"
fi

# Autonomous workers must recover abandoned edit leases promptly. Interactive
# editors retain the conservative claim-guard default; the perpetual conductor
# can safely use a shorter bounded lease because its worker heartbeat refreshes
# active claims and the watchdog handles genuinely live conflicts.
export AUTOSPEC_CLAIM_TTL_SECONDS="${AUTOSPEC_CLAIM_TTL_SECONDS:-600}"

if ! command -v omx >/dev/null 2>&1; then
    printf 'autospec-autonomous-run-drain: omx not found on PATH\n' >&2
    exit 127
fi

stat_size() {
    stat -c '%s' /dev/fd/1 2>/dev/null || stat -f '%z' /dev/fd/1 2>/dev/null || printf ''
}

stat_file_signature() {
    _file="$1"
    [ -f "$_file" ] || return 0
    _size="$(stat -c '%s' "$_file" 2>/dev/null || stat -f '%z' "$_file" 2>/dev/null || printf '')"
    _mtime="$(stat -c '%Y' "$_file" 2>/dev/null || stat -f '%m' "$_file" 2>/dev/null || printf '')"
    printf '%s:%s:%s\n' "$_file" "$_size" "$_mtime"
}

progress_file_candidates() {
    if [ -n "${AUTOSPEC_AUTONOMOUS_DRAIN_LOG:-}" ]; then
        printf '%s\n' "$AUTOSPEC_AUTONOMOUS_DRAIN_LOG"
    fi
    if [ -n "${AUTOSPEC_AUTONOMOUS_DRAIN_LOG_FILE:-}" ]; then
        printf '%s\n' "$AUTOSPEC_AUTONOMOUS_DRAIN_LOG_FILE"
    fi
    if [ -n "${AUTOSPEC_AUTONOMOUS_DRAIN_LOG_GLOB:-}" ]; then
        for _candidate in $AUTOSPEC_AUTONOMOUS_DRAIN_LOG_GLOB; do
            [ -e "$_candidate" ] && printf '%s\n' "$_candidate"
        done
    fi
    if [ -d "$HOME/.autospec/process-heartbeats" ]; then
        find "$HOME/.autospec/process-heartbeats" -type f -name '*.json' -print 2>/dev/null || true
    fi
    closeout_file_candidates
}

drain_issue_number() {
    if [ -n "${AUTOSPEC_AUTONOMOUS_DRAIN_ISSUE:-}" ]; then
        printf '%s\n' "$AUTOSPEC_AUTONOMOUS_DRAIN_ISSUE"
        return 0
    fi
    if [ -n "${AUTOSPEC_ISSUE_NUMBER:-}" ]; then
        printf '%s\n' "$AUTOSPEC_ISSUE_NUMBER"
        return 0
    fi
    return 1
}

closeout_file_candidates() {
    _issue="$(drain_issue_number 2>/dev/null || true)"
    if [ -n "${AUTOSPEC_AUTONOMOUS_DRAIN_CLOSEOUT_ARTIFACTS:-}" ]; then
        for _candidate in $AUTOSPEC_AUTONOMOUS_DRAIN_CLOSEOUT_ARTIFACTS; do
            [ -n "$_candidate" ] && printf '%s\n' "$_candidate"
        done
    fi
    [ -n "$_issue" ] || return 0
    printf '%s\n' "$REPO_DIR/.autospec/run-summary.md"
    printf '%s\n' "/tmp/write-summary-${_issue}.log"
    printf '%s\n' "/tmp/autospec-run-${_issue}/done-challenge.md"
}

progress_signature() {
    printf 'stdout:%s\n' "$(stat_size)"
    progress_file_candidates | sort -u | while IFS= read -r _candidate; do
        [ -n "$_candidate" ] || continue
        stat_file_signature "$_candidate"
    done
}

kill_tree() {
    _pid="$1"
    for _child in $(pgrep -P "$_pid" 2>/dev/null || true); do
        kill_tree "$_child"
    done
    kill "$_pid" 2>/dev/null || true
    # A shell blocked in `wait` may defer TERM until its child exits. Escalate
    # immediately after recursively signalling descendants so stall recovery
    # never inherits the child's full sleep/runtime.
    kill -KILL "$_pid" 2>/dev/null || true
}

child_is_running() {
    jobs -pr | grep -qx "$child_pid" || has_live_descendant "$child_pid"
}

has_live_descendant() {
    _pid="$1"
    for _child in $(pgrep -P "$_pid" 2>/dev/null || true); do
        if kill -0 "$_child" 2>/dev/null; then
            return 0
        fi
        if has_live_descendant "$_child"; then
            return 0
        fi
    done
    return 1
}

detect_repo() {
    if [ -n "${CONDUCTOR_REPO:-}" ]; then
        printf '%s\n' "$CONDUCTOR_REPO"
        return 0
    fi
    if [ -n "${AUTOSPEC_REPO:-}" ]; then
        printf '%s\n' "$AUTOSPEC_REPO"
        return 0
    fi
    gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true
}

repo_heartbeat_dirs() {
    _repo="$1"
    [ -n "$_repo" ] || return 0
    _base="${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"
    _owner="${_repo%%/*}"
    _name="${_repo##*/}"
    printf '%s/%s__%s\n' "$_base" "$_owner" "$_name"
    printf '%s/%s_%s\n' "$_base" "$_owner" "$_name"
    printf '%s/%s-%s\n' "$_base" "$_owner" "$_name"
}

newest_heartbeat_mtime() {
    _repo="$1"
    _newest=0
    for _dir in $(repo_heartbeat_dirs "$_repo"); do
        [ -d "$_dir" ] || continue
        for _file in "$_dir"/*.json; do
            [ -f "$_file" ] || continue
            _mtime="$(stat -c %Y "$_file" 2>/dev/null || stat -f %m "$_file" 2>/dev/null || printf '0')"
            case "$_mtime" in *[!0-9]*|'') _mtime=0 ;; esac
            if [ "$_mtime" -gt "$_newest" ]; then
                _newest="$_mtime"
            fi
        done
    done
    printf '%s\n' "$_newest"
}

issue_has_in_progress_label() {
    _repo="$1"
    _issue="$2"
    _json="$(gh issue view "$_issue" --repo "$_repo" --json state,labels 2>/dev/null || true)"
    [ -n "$_json" ] || return 1
    printf '%s' "$_json" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
labels = {item.get("name") for item in data.get("labels", []) if isinstance(item, dict)}
if data.get("state") == "OPEN" and "in-progress-by-bot" in labels:
    sys.exit(0)
sys.exit(1)
'
}

green_issue_pr_candidates() {
    _repo="$1"
    _json="$(gh pr list --repo "$_repo" --state open --json number,headRefName,statusCheckRollup,isDraft 2>/dev/null || true)"
    [ -n "$_json" ] || return 0
    printf '%s' "$_json" | python3 -c '
import json, re, sys
try:
    prs = json.load(sys.stdin)
except Exception:
    sys.exit(0)

def check_complete(check):
    status = str(check.get("status") or check.get("state") or "").upper()
    conclusion = str(check.get("conclusion") or check.get("state") or "").upper()
    if status and status not in {"COMPLETED", "SUCCESS"}:
        return False
    if conclusion and conclusion not in {"SUCCESS", "SKIPPED", "NEUTRAL"}:
        return False
    return True

for pr in prs:
    if pr.get("isDraft"):
        continue
    head = str(pr.get("headRefName") or "")
    match = re.search(r"issue-([0-9]+)(?:-|$)", head)
    if not match:
        continue
    checks = pr.get("statusCheckRollup") or []
    if not checks or not all(isinstance(check, dict) and check_complete(check) for check in checks):
        continue
    print("{} {}".format(pr.get("number"), match.group(1)))
'
}

pr_is_merged() {
    _repo="$1"
    _pr="$2"
    _json="$(gh pr view "$_pr" --repo "$_repo" --json state,mergedAt 2>/dev/null || true)"
    [ -n "$_json" ] || return 1
    printf '%s' "$_json" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
if data.get("state") == "MERGED" or data.get("mergedAt"):
    sys.exit(0)
sys.exit(1)
'
}

recover_green_in_progress_pr() {
    command -v gh >/dev/null 2>&1 || return 1
    command -v python3 >/dev/null 2>&1 || return 1
    _repo="$(detect_repo)"
    [ -n "$_repo" ] || return 1

    _candidates="$(green_issue_pr_candidates "$_repo")"
    [ -n "$_candidates" ] || return 1
    while read -r _pr _issue; do
        [ -n "${_pr:-}" ] && [ -n "${_issue:-}" ] || continue
        if ! issue_has_in_progress_label "$_repo" "$_issue"; then
            continue
        fi
        gh pr merge "$_pr" --repo "$_repo" --admin --squash --delete-branch >/dev/null 2>&1 || true
        if pr_is_merged "$_repo" "$_pr"; then
            gh issue edit "$_issue" --repo "$_repo" --remove-label in-progress-by-bot >/dev/null 2>&1 || true
            printf 'autospec-autonomous-run-drain: stale wait handle recovery merged PR #%s for issue #%s\n' "$_pr" "$_issue"
            return 0
        fi
    done <<EOF
$_candidates
EOF
    return 1
}

record_closeout_hang() {
    _issue="$(drain_issue_number 2>/dev/null || true)"
    [ -n "$_issue" ] || return 1
    _dir="${AUTOSPEC_AUTONOMOUS_DRAIN_CLOSEOUT_DIR:-/tmp/autospec-run-${_issue}}"
    mkdir -p "$_dir"
    _artifact="$_dir/closeout-hang.md"
    _now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    cat > "$_artifact" <<EOF
# autospec closeout hang

Issue #${_issue} hit a closeout hang at ${_now}: no drain output, no heartbeat/log/summary artifact progress, and no live worker descendant was detected.
EOF
    printf 'autospec-autonomous-run-drain: closeout hang for issue #%s; wrote %s\n' "$_issue" "$_artifact" >&2
    if command -v gh >/dev/null 2>&1; then
        _repo="$(detect_repo)"
        if [ -n "$_repo" ]; then
            gh issue comment "$_issue" --repo "$_repo" --body-file "$_artifact" >/dev/null 2>&1 || true
        fi
    fi
    return 0
}

# Provide a literal, shell-safe resume command so autospec-run preflight does
# not interpolate `$autospec-run` as an unset shell variable under `set -u`.
export AUTOSPEC_RESUME_COMMAND="${AUTOSPEC_RESUME_COMMAND:-cd $(printf '%q' "$REPO_DIR") && omx exec '\$autospec-run'}"

omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    '$autospec-run' &
child_pid="$!"

if [ "${DRAIN_STALL_SECS:-0}" -le 0 ] 2>/dev/null; then
    wait "$child_pid"
    exit "$?"
fi

last_progress_signature="$(progress_signature)"
last_progress_epoch="$(date +%s)"
detected_repo="$(detect_repo)"
last_heartbeat_mtime="$(newest_heartbeat_mtime "$detected_repo")"

while child_is_running; do
    sleep "$DRAIN_POLL_SECS"
    child_is_running || break
    current_progress_signature="$(progress_signature)"
    if [ -n "$current_progress_signature" ] && [ "$current_progress_signature" != "$last_progress_signature" ]; then
        last_progress_signature="$current_progress_signature"
        last_progress_epoch="$(date +%s)"
        continue
    fi
    current_heartbeat_mtime="$(newest_heartbeat_mtime "$detected_repo")"
    if [ "$current_heartbeat_mtime" -gt "$last_heartbeat_mtime" ]; then
        last_heartbeat_mtime="$current_heartbeat_mtime"
        last_progress_epoch="$(date +%s)"
        continue
    fi
    now_epoch="$(date +%s)"
    idle_secs=$((now_epoch - last_progress_epoch))
    if [ "$idle_secs" -ge "$DRAIN_STALL_SECS" ]; then
        if has_live_descendant "$child_pid" && recover_green_in_progress_pr; then
            exit 0
        fi
        record_closeout_hang || true
        printf 'autospec-autonomous-run-drain: stalled after %ss with no output; terminating autospec-run child pid %s\n' \
            "$DRAIN_STALL_SECS" "$child_pid" >&2
        kill_tree "$child_pid"
        wait "$child_pid" 2>/dev/null || true
        if recover_green_in_progress_pr; then
            exit 0
        fi
        exit 124
    fi
done

set +e
wait "$child_pid"
child_status="$?"
set -e
if [ "$child_status" -ne 0 ] && recover_green_in_progress_pr; then
    exit 0
fi
exit "$child_status"
