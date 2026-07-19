#!/usr/bin/env bash
# autospec-run-events.sh — append-only run event recorder plus explain/replay.
set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-run-events.sh record --events FILE --repo OWNER/REPO --run-id ID --event NAME --decision NAME --reason TEXT [--issue N] [--pr N]
  autospec-run-events.sh explain --events FILE
  autospec-run-events.sh replay --events FILE
EOF
}

die() {
    printf 'autospec-run-events: %s\n' "$*" >&2
    exit 2
}

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()), end="")'
}

cmd="${1:-}"
[ -n "$cmd" ] || { usage; exit 2; }
shift

events=""
repo=""
run_id=""
event=""
decision=""
reason=""
issue="null"
pr="null"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --events) events="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --run-id) run_id="${2:-}"; shift 2 ;;
        --event) event="${2:-}"; shift 2 ;;
        --decision) decision="${2:-}"; shift 2 ;;
        --reason) reason="${2:-}"; shift 2 ;;
        --issue) issue="${2:-}"; shift 2 ;;
        --pr) pr="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

case "$cmd" in
    record)
        [ -n "$events" ] || die "--events is required"
        [ -n "$repo" ] || die "--repo is required"
        [ -n "$run_id" ] || die "--run-id is required"
        [ -n "$event" ] || die "--event is required"
        [ -n "$decision" ] || die "--decision is required"
        [ -n "$reason" ] || die "--reason is required"
        mkdir -p "$(dirname "$events")"
        ts="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
        jq -cn \
            --arg ts "$ts" \
            --arg repo "$repo" \
            --arg run_id "$run_id" \
            --arg event "$event" \
            --arg decision "$decision" \
            --arg reason "$reason" \
            --arg issue "$issue" \
            --arg pr "$pr" \
            '{ts:$ts,repo:$repo,run_id:$run_id,event:$event,decision:$decision,reason:$reason,issue:(if $issue=="null" then null else ($issue|tonumber? // $issue) end),pr:(if $pr=="null" then null else ($pr|tonumber? // $pr) end)}' \
            >> "$events"
        ;;
    explain)
        [ -n "$events" ] || die "--events is required"
        [ -f "$events" ] || die "events file not found: $events"
        jq -s -r '
          if length == 0 then "No run events recorded."
          else
            .[-1] as $last |
            "Run: \($last.run_id // "unknown")\n" +
            "Repo: \($last.repo // "unknown")\n" +
            "Final decision: \($last.decision // "unknown")\n" +
            "Reason: \($last.reason // "unknown")\n" +
            (if $last.issue then "Issue: #\($last.issue)\n" else "" end) +
            (if $last.pr then "PR: #\($last.pr)\n" else "" end)
          end' "$events"
        ;;
    replay)
        [ -n "$events" ] || die "--events is required"
        [ -f "$events" ] || die "events file not found: $events"
        jq -s '
          if length == 0 then
            {events:0, final_decision:null, reason:null, issue:null, pr:null}
          else
            .[-1] as $last |
            {events:length, run_id:$last.run_id, repo:$last.repo, final_decision:$last.decision, reason:$last.reason, issue:$last.issue, pr:$last.pr}
          end' "$events"
        ;;
    *)
        die "unknown command: $cmd"
        ;;
esac

