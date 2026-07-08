#!/usr/bin/env bash
# autospec-autonomous-run-drain.sh — one Tier-1 drain invocation for the conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${AUTOSPEC_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
DRAIN_STALL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS:-1800}"
DRAIN_POLL_SECS="${AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS:-15}"

if ! command -v omx >/dev/null 2>&1; then
    printf 'autospec-autonomous-run-drain: omx not found on PATH\n' >&2
    exit 127
fi

stat_size() {
    stat -f '%z' /dev/fd/1 2>/dev/null || stat -c '%s' /dev/fd/1 2>/dev/null || printf ''
}

kill_tree() {
    _pid="$1"
    for _child in $(pgrep -P "$_pid" 2>/dev/null || true); do
        kill_tree "$_child"
    done
    kill "$_pid" 2>/dev/null || true
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

omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    '$autospec-run' &
child_pid="$!"

if [ "${DRAIN_STALL_SECS:-0}" -le 0 ] 2>/dev/null; then
    wait "$child_pid"
    exit "$?"
fi

last_size="$(stat_size)"
last_progress_epoch="$(date +%s)"

while kill -0 "$child_pid" 2>/dev/null; do
    sleep "$DRAIN_POLL_SECS"
    current_size="$(stat_size)"
    if [ -n "$current_size" ] && [ "$current_size" != "$last_size" ]; then
        last_size="$current_size"
        last_progress_epoch="$(date +%s)"
        continue
    fi
    now_epoch="$(date +%s)"
    idle_secs=$((now_epoch - last_progress_epoch))
    if [ "$idle_secs" -ge "$DRAIN_STALL_SECS" ]; then
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
