#!/usr/bin/env bash
# scripts/dogfood-control-plane.sh — dogfood the control-plane MVP flow.
#
# Produces deterministic local evidence that autospec can bootstrap companion
# repos, keep working with the observatory offline, replay the outbox, and derive
# timeline/cost artifacts for a berlinguyinca/autospec run.

set -euo pipefail

PROG="dogfood-control-plane.sh"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTROL_PLANE="$SCRIPT_DIR/autospec-control-plane.sh"
EVENTS="$SCRIPT_DIR/autospec-observatory-events.sh"

usage() {
  cat <<'USAGE'
Usage:
  scripts/dogfood-control-plane.sh [--offline|--online] --run-id ID --output-dir DIR [options]
  scripts/dogfood-control-plane.sh --replay-only [--offline|--online] --run-id ID --output-dir DIR [options]

Options:
  --offline                 Force offline replay; never requires the observatory service (default).
  --online                  Attempt upload to AUTOSPEC_OBSERVATORY_URL or --observatory-url.
  --observatory-url URL     Observatory base URL or /v1/events/batch endpoint for online replay.
  --run-id ID               Stable dogfood run id (default: control-plane-dogfood-<timestamp>).
  --output-dir DIR          Artifact directory (default: .autospec/control-plane-dogfood/<run-id>).
  --replay-only             Rebuild timeline.json and cost-report.json from an existing outbox.
  --owner OWNER             Companion repo owner for bootstrap dry-run (default: berlinguyinca).
  --governance-repo NAME    Governance companion repo name (default: autospec-governance).
  --observatory-repo NAME   Observatory companion repo name (default: autospec-observatory).
  --repository-id OWNER/REPO Repository id to record in events (default: berlinguyinca/autospec).
  --worker-id ID            Worker id for dogfood events (default: AUTOSPEC_WORKER_ID or local pid).
  --help                    Show this help.

Artifacts:
  companion-bootstrap.txt   Dry-run companion bootstrap scaffold output.
  outbox.jsonl              Copy of the local observatory outbox used for replay.
  replay.log                Offline/online replay status from autospec-observatory-events.sh.
  timeline.json             Ordered run timeline derived from replayed outbox events.
  cost-report.json          Cost/duration/outcome evidence derived from replayed outbox events.
  manifest.json             Paths and run metadata for operator verification.

This script is intentionally safe for CI: --offline is the default, bootstrap is
run with --dry-run, and replay uses the local durable outbox before any upload.
USAGE
}

fail() {
  printf '%s: %s\n' "$PROG" "$*" >&2
  exit 2
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }
default_run_id() { date -u +control-plane-dogfood-%Y%m%dT%H%M%SZ; }

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

json_quote() {
  jq -Rn --arg v "$1" '$v'
}

mode="offline"
run_id=""
output_dir=""
replay_only=0
owner="berlinguyinca"
governance_repo="autospec-governance"
observatory_repo="autospec-observatory"
repository_id="berlinguyinca/autospec"
worker_id="${AUTOSPEC_WORKER_ID:-dogfood-$USER-$$}"
observatory_url="${AUTOSPEC_OBSERVATORY_URL:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --offline) mode="offline"; shift ;;
    --online) mode="online"; shift ;;
    --observatory-url) [ "$#" -ge 2 ] || fail "--observatory-url requires a value"; observatory_url="$2"; shift 2 ;;
    --run-id) [ "$#" -ge 2 ] || fail "--run-id requires a value"; run_id="$2"; shift 2 ;;
    --output-dir) [ "$#" -ge 2 ] || fail "--output-dir requires a value"; output_dir="$2"; shift 2 ;;
    --replay-only) replay_only=1; shift ;;
    --owner) [ "$#" -ge 2 ] || fail "--owner requires a value"; owner="$2"; shift 2 ;;
    --governance-repo) [ "$#" -ge 2 ] || fail "--governance-repo requires a value"; governance_repo="$2"; shift 2 ;;
    --observatory-repo) [ "$#" -ge 2 ] || fail "--observatory-repo requires a value"; observatory_repo="$2"; shift 2 ;;
    --repository-id) [ "$#" -ge 2 ] || fail "--repository-id requires a value"; repository_id="$2"; shift 2 ;;
    --worker-id) [ "$#" -ge 2 ] || fail "--worker-id requires a value"; worker_id="$2"; shift 2 ;;
    --help|-h|help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[ -n "$run_id" ] || run_id="$(default_run_id)"
[ -n "$output_dir" ] || output_dir="$REPO_ROOT/.autospec/control-plane-dogfood/$run_id"

require_command jq
[ -f "$CONTROL_PLANE" ] || fail "missing $CONTROL_PLANE"
[ -f "$EVENTS" ] || fail "missing $EVENTS"
mkdir -p "$output_dir"

export AUTOSPEC_RUN_ID="$run_id"
export AUTOSPEC_WORKER_ID="$worker_id"
export AUTOSPEC_REPOSITORY_ID="$repository_id"
if [ "$mode" = "offline" ]; then
  export AUTOSPEC_OBSERVATORY_OFFLINE=1
else
  unset AUTOSPEC_OBSERVATORY_OFFLINE
  [ -n "$observatory_url" ] || fail "--online requires --observatory-url or AUTOSPEC_OBSERVATORY_URL"
  export AUTOSPEC_OBSERVATORY_URL="$observatory_url"
fi

if [ "$replay_only" -eq 0 ]; then
  bash "$CONTROL_PLANE" bootstrap --dry-run \
    --owner "$owner" \
    --governance-repo "$governance_repo" \
    --observatory-repo "$observatory_repo" \
    > "$output_dir/companion-bootstrap.txt"

  bash "$EVENTS" emit --run-id "$run_id" --event-id "$run_id-run-started" \
    --event-type RunStarted --repository-id "$repository_id" --worker-id "$worker_id" \
    --status running --summary "dogfood run started" --progress-phase bootstrap --progress-percent 5 \
    --current-item-title "bootstrap companion repositories" --planned-next-step "run autospec offline" >/dev/null
  bash "$EVENTS" emit --run-id "$run_id" --event-id "$run_id-bootstrap-completed" \
    --event-type ControlPlaneBootstrapCompleted --repository-id "$repository_id" --worker-id "$worker_id" \
    --status completed --summary "companion bootstrap dry-run completed" --progress-phase bootstrap --progress-percent 25 \
    --current-item-title "autospec-governance + autospec-observatory dry-run" --planned-next-step "emit offline outbox events" >/dev/null
  bash "$EVENTS" emit --run-id "$run_id" --event-id "$run_id-work-started" \
    --event-type WorkItemStarted --repository-id "$repository_id" --worker-id "$worker_id" \
    --status running --summary "autospec issue #1621 dogfood work item started" --progress-phase autospec-run --progress-percent 50 \
    --current-item-title "Dogfood end-to-end control-plane flow" --current-item-url "https://github.com/$repository_id/issues/1621" \
    --planned-next-step "capture timeline and cost" >/dev/null
  bash "$EVENTS" emit --run-id "$run_id" --event-id "$run_id-cost-reported" \
    --event-type CostReported --repository-id "$repository_id" --worker-id "$worker_id" \
    --status completed --summary "estimated dogfood smoke cost recorded" --progress-phase reports --progress-percent 75 \
    --current-item-title "cost report" --planned-next-step "replay outbox" >/dev/null
  bash "$EVENTS" emit --run-id "$run_id" --event-id "$run_id-run-completed" \
    --event-type RunCompleted --repository-id "$repository_id" --worker-id "$worker_id" \
    --status completed --summary "dogfood run completed" --progress-phase complete --progress-percent 100 \
    --current-item-title "timeline and cost artifacts" --planned-next-step "operator verification" >/dev/null
else
  [ -f "$output_dir/companion-bootstrap.txt" ] || printf 'replay-only: companion bootstrap was generated by a prior run\n' > "$output_dir/companion-bootstrap.txt"
fi

# Replay the durable outbox. Offline mode records STATUS:offline; online mode
# attempts upload and records uploaded/queued status without deleting evidence.
bash "$EVENTS" flush --run-id "$run_id" > "$output_dir/replay.log"

outbox="${AUTOSPEC_OBSERVATORY_DIR:-.autospec/observatory}/outbox/$run_id.jsonl"
[ -s "$outbox" ] || fail "outbox is empty or missing: $outbox"
cp "$outbox" "$output_dir/outbox.jsonl"

replay_mode="full"
[ "$replay_only" -eq 0 ] || replay_mode="replay-only"

jq -s \
  --arg run_id "$run_id" \
  --arg repository_id "$repository_id" \
  --arg replay_mode "$replay_mode" \
  '{run_id:$run_id, repository_id:$repository_id, replay_mode:$replay_mode, generated_at:(now|strftime("%Y-%m-%dT%H:%M:%SZ")), events:(sort_by(.sequence) | map({sequence,event_id,event_type,status,summary,progress_phase,progress_percent,current_item_title,current_item_url,planned_next_step,occurred_at,worker_id}))}' \
  "$outbox" > "$output_dir/timeline.json"

jq -s \
  --arg run_id "$run_id" \
  --arg repository_id "$repository_id" \
  '{run_id:$run_id, repository_id:$repository_id, generated_at:(now|strftime("%Y-%m-%dT%H:%M:%SZ")), total_events:length, estimated_cost_usd:([.[].estimated_cost_usd // 0] | add), actual_cost_usd:([.[].actual_cost_usd // 0] | add), cost_events:([.[] | select(.event_type == "CostReported")] | length), first_event_at:([.[].occurred_at] | min), last_event_at:([.[].occurred_at] | max), outcomes:([.[] | select(.status != null) | .status] | unique)}' \
  "$outbox" > "$output_dir/cost-report.json"

jq -n \
  --arg run_id "$run_id" \
  --arg mode "$mode" \
  --arg replay_mode "$replay_mode" \
  --arg repository_id "$repository_id" \
  --arg bootstrap "$output_dir/companion-bootstrap.txt" \
  --arg outbox "$output_dir/outbox.jsonl" \
  --arg replay "$output_dir/replay.log" \
  --arg timeline "$output_dir/timeline.json" \
  --arg cost "$output_dir/cost-report.json" \
  --arg generated_at "$(now_iso)" \
  '{run_id:$run_id, mode:$mode, replay_mode:$replay_mode, repository_id:$repository_id, generated_at:$generated_at, artifacts:{companion_bootstrap:$bootstrap,outbox:$outbox,replay_log:$replay,timeline:$timeline,cost_report:$cost}}' \
  > "$output_dir/manifest.json"

cat "$output_dir/replay.log"
printf 'replay_mode=%s\n' "$replay_mode"
printf 'artifact_dir=%s\n' "$output_dir"
printf 'timeline_artifact=%s\n' "$output_dir/timeline.json"
printf 'cost_artifact=%s\n' "$output_dir/cost-report.json"
printf 'manifest_artifact=%s\n' "$output_dir/manifest.json"
