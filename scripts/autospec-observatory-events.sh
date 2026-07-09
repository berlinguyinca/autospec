#!/usr/bin/env bash
# scripts/autospec-observatory-events.sh — local observatory event outbox helper.
#
# Writes structured autospec events to a durable local JSONL outbox before any
# upload attempt. Upload is best-effort and offline-safe: helper commands never
# fail solely because the observatory service is absent.

set -euo pipefail

PROG="autospec-observatory-events.sh"

usage() {
  cat <<'USAGE'
Usage:
  autospec-observatory-events.sh emit --run-id ID --event-type TYPE [options]
  autospec-observatory-events.sh dry-run --run-id ID [--worker-id ID] [options]
  autospec-observatory-events.sh flush --run-id ID
  autospec-observatory-events.sh status --run-id ID

Options:
  --event-id ID              Stable event id for dedupe (default generated)
  --worker-id ID             Worker id (default AUTOSPEC_WORKER_ID)
  --agent-id ID              Agent id
  --repository-id OWNER/REPO Repository id
  --issue-url URL            Current issue URL
  --pr-url URL               Current PR URL
  --commit-sha SHA           Current commit SHA
  --status STATUS            Event status
  --summary TEXT             Event summary
  --progress-phase TEXT      Normalized progress phase
  --progress-percent N       Progress percentage
  --current-item-title TEXT  Current item title
  --current-item-url URL     Current item URL
  --planned-next-step TEXT   Planned next operator-visible step

Environment:
  AUTOSPEC_OBSERVATORY_DIR       Outbox root (default .autospec/observatory)
  AUTOSPEC_OBSERVATORY_OFFLINE=1 Force offline mode; never upload
  AUTOSPEC_OBSERVATORY_URL       Observatory base URL or /v1/events/batch URL
USAGE
}

fail_usage() {
  printf '%s: %s\n' "$PROG" "$*" >&2
  usage >&2
  exit 2
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }

retry_iso() {
  seconds="$1"
  date -u -r "$(( $(date -u +%s) + seconds ))" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
    || date -u -d "+${seconds} seconds" +'%Y-%m-%dT%H:%M:%SZ'
}

generate_event_id() {
  if command -v uuidgen >/dev/null 2>&1; then
    uuidgen | tr '[:upper:]' '[:lower:]'
  else
    printf 'evt-%s-%s' "$(date -u +%Y%m%dT%H%M%SZ)" "$$"
  fi
}

observatory_dir() { printf '%s' "${AUTOSPEC_OBSERVATORY_DIR:-.autospec/observatory}"; }
outbox_dir() { printf '%s/outbox' "$(observatory_dir)"; }
checkpoint_file() { printf '%s/checkpoints.json' "$(observatory_dir)"; }
outbox_file() { printf '%s/%s.jsonl' "$(outbox_dir)" "$1"; }

ensure_storage() {
  mkdir -p "$(outbox_dir)"
  if [ ! -f "$(checkpoint_file)" ]; then
    printf '{}\n' > "$(checkpoint_file)"
  fi
}

json_string_or_null() {
  value="$1"
  if [ -n "$value" ]; then
    jq -Rn --arg v "$value" '$v'
  else
    printf 'null'
  fi
}

checkpoint_value() {
  run_id="$1"
  key="$2"
  default="$3"
  jq -r --arg run "$run_id" --arg key "$key" --arg default "$default" \
    '.[$run][$key] // $default' "$(checkpoint_file)"
}

next_sequence() {
  run_id="$1"
  checkpoint_value "$run_id" next_sequence 1
}

update_checkpoint_after_emit() {
  run_id="$1"
  sequence="$2"
  outbox="$3"
  tmp="$(mktemp "$(observatory_dir)/checkpoints.XXXXXX")"
  jq --arg run "$run_id" \
     --arg outbox "$outbox" \
     --arg updated_at "$(now_iso)" \
     --argjson last_sequence "$sequence" \
     --argjson next_sequence "$((sequence + 1))" \
     '.[$run] = ((.[$run] // {}) + {
        last_sequence: $last_sequence,
        next_sequence: $next_sequence,
        outbox: $outbox,
        upload_status: (.[$run].upload_status // "pending"),
        updated_at: $updated_at
      })' "$(checkpoint_file)" > "$tmp"
  mv "$tmp" "$(checkpoint_file)"
}

update_upload_checkpoint() {
  run_id="$1"
  status="$2"
  retry_count="$3"
  backoff_seconds="$4"
  tmp="$(mktemp "$(observatory_dir)/checkpoints.XXXXXX")"
  if [ "$backoff_seconds" -gt 0 ]; then
    next_retry_at="$(retry_iso "$backoff_seconds")"
  else
    next_retry_at=""
  fi
  jq --arg run "$run_id" \
     --arg status "$status" \
     --arg updated_at "$(now_iso)" \
     --arg next_retry_at "$next_retry_at" \
     --argjson retry_count "$retry_count" \
     --argjson backoff_seconds "$backoff_seconds" \
     '.[$run] = ((.[$run] // {}) + {
        upload_status: $status,
        retry_count: $retry_count,
        backoff_seconds: $backoff_seconds,
        next_retry_at: $next_retry_at,
        updated_at: $updated_at
      })' "$(checkpoint_file)" > "$tmp"
  mv "$tmp" "$(checkpoint_file)"
}

has_event_id() {
  file="$1"
  event_id="$2"
  [ -f "$file" ] || return 1
  jq -e --arg event_id "$event_id" 'select(.event_id == $event_id)' "$file" >/dev/null 2>&1
}

reset_emit_args() {
  run_id="${AUTOSPEC_RUN_ID:-}"
  event_type=""
  event_id=""
  worker_id="${AUTOSPEC_WORKER_ID:-}"
  agent_id="${AUTOSPEC_AGENT_ID:-}"
  harness="${AUTOSPEC_HARNESS:-}"
  model="${AUTOSPEC_MODEL:-}"
  skill_or_workflow="${AUTOSPEC_SKILL_OR_WORKFLOW:-autospec-run}"
  repository_id="${AUTOSPEC_REPOSITORY_ID:-}"
  issue_url="${AUTOSPEC_ISSUE_URL:-}"
  pr_url="${AUTOSPEC_PR_URL:-}"
  commit_sha="${AUTOSPEC_COMMIT_SHA:-}"
  status=""
  summary=""
  progress_phase=""
  progress_percent=""
  current_item_title=""
  current_item_url=""
  planned_next_step=""
}

parse_emit_args() {
  reset_emit_args
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --run-id) run_id="${2:-}"; shift 2 ;;
      --event-type) event_type="${2:-}"; shift 2 ;;
      --event-id) event_id="${2:-}"; shift 2 ;;
      --worker-id) worker_id="${2:-}"; shift 2 ;;
      --agent-id) agent_id="${2:-}"; shift 2 ;;
      --harness) harness="${2:-}"; shift 2 ;;
      --model) model="${2:-}"; shift 2 ;;
      --skill-or-workflow) skill_or_workflow="${2:-}"; shift 2 ;;
      --repository-id) repository_id="${2:-}"; shift 2 ;;
      --issue-url) issue_url="${2:-}"; shift 2 ;;
      --pr-url) pr_url="${2:-}"; shift 2 ;;
      --commit-sha) commit_sha="${2:-}"; shift 2 ;;
      --status) status="${2:-}"; shift 2 ;;
      --summary) summary="${2:-}"; shift 2 ;;
      --progress-phase) progress_phase="${2:-}"; shift 2 ;;
      --progress-percent) progress_percent="${2:-}"; shift 2 ;;
      --current-item-title) current_item_title="${2:-}"; shift 2 ;;
      --current-item-url) current_item_url="${2:-}"; shift 2 ;;
      --planned-next-step) planned_next_step="${2:-}"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) fail_usage "unknown emit option: $1" ;;
    esac
  done
  [ -n "$run_id" ] || fail_usage "emit requires --run-id"
  [ -n "$event_type" ] || fail_usage "emit requires --event-type"
  [ -n "$event_id" ] || event_id="$(generate_event_id)"
}

append_event_json() {
  jq -cn \
    --arg event_id "$event_id" --arg run_id "$run_id" --arg event_type "$event_type" \
    --arg occurred_at "$occurred_at" --arg received_at "$occurred_at" \
    --arg worker_id "$worker_id" --arg agent_id "$agent_id" --arg harness "$harness" \
    --arg model "$model" --arg skill_or_workflow "$skill_or_workflow" \
    --arg repository_id "$repository_id" --arg issue_url "$issue_url" --arg pr_url "$pr_url" \
    --arg commit_sha "$commit_sha" --arg status "$status" --arg summary "$summary" \
    --arg progress_phase "$progress_phase" --arg progress_percent "$progress_percent" \
    --arg current_item_title "$current_item_title" --arg current_item_url "$current_item_url" \
    --arg planned_next_step "$planned_next_step" --argjson sequence "$sequence" \
    '{event_id:$event_id,event_type:$event_type,run_id:$run_id,sequence:$sequence,occurred_at:$occurred_at,received_at:$received_at,
      org_id:null,workspace_id:null,project_id:null,repository_id:($repository_id|select(length>0) // null),
      project_classification:null,privacy_tier:null,operator_id:null,worker_id:($worker_id|select(length>0) // null),
      agent_id:($agent_id|select(length>0) // null),harness:($harness|select(length>0) // null),model:($model|select(length>0) // null),
      skill_or_workflow:($skill_or_workflow|select(length>0) // null),issue_url:($issue_url|select(length>0) // null),
      pr_url:($pr_url|select(length>0) // null),commit_sha:($commit_sha|select(length>0) // null),policy_id:null,policy_version:null,
      policy_digest:null,duration_ms:null,estimated_cost_usd:null,actual_cost_usd:null,risk_level:null,status:($status|select(length>0) // null),
      summary:($summary|select(length>0) // null),progress_percent:(if $progress_percent == "" then null else ($progress_percent|tonumber) end),
      progress_phase:($progress_phase|select(length>0) // null),current_item_title:($current_item_title|select(length>0) // null),
      current_item_url:($current_item_url|select(length>0) // null),queue_ready_count:null,queue_blocked_count:null,queue_claimed_count:null,
      queue_remaining_count:null,estimated_next_item_at:null,estimated_completion_at:null,planned_next_step:($planned_next_step|select(length>0) // null),
      artifact_links:[]}' >> "$outbox"
}

emit_event() {
  parse_emit_args "$@"
  ensure_storage
  outbox="$(outbox_file "$run_id")"
  if has_event_id "$outbox" "$event_id"; then
    printf 'STATUS:deduped run_id=%s event_id=%s\n' "$run_id" "$event_id"
    return 0
  fi
  sequence="$(next_sequence "$run_id")"
  occurred_at="$(now_iso)"
  append_event_json
  update_checkpoint_after_emit "$run_id" "$sequence" "$outbox"
  printf 'STATUS:emitted run_id=%s event_type=%s sequence=%s outbox=%s\n' "$run_id" "$event_type" "$sequence" "$outbox"
}

parse_run_id_args() {
  parsed_run_id="${AUTOSPEC_RUN_ID:-}"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --run-id) parsed_run_id="${2:-}"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) fail_usage "unknown $1 option: $1" ;;
    esac
  done
  [ -n "$parsed_run_id" ] || fail_usage "command requires --run-id"
}

pending_events() {
  if [ -f "$1" ]; then
    wc -l < "$1" | tr -d ' '
  else
    printf '0'
  fi
}

queue_upload() {
  run_id="$1"
  pending="$2"
  retry_count="$(checkpoint_value "$run_id" retry_count 0)"
  retry_count="$((retry_count + 1))"
  backoff_seconds="$((2 ** retry_count))"
  [ "$backoff_seconds" -le 300 ] || backoff_seconds=300
  update_upload_checkpoint "$run_id" "queued" "$retry_count" "$backoff_seconds"
  printf 'STATUS:queued run_id=%s pending_events=%s retry_count=%s backoff_seconds=%s\n' "$run_id" "$pending" "$retry_count" "$backoff_seconds"
}

post_payload() {
  payload="$1"
  endpoint="$2"
  command -v curl >/dev/null 2>&1 || return 1
  curl -fsS -X POST -H 'Content-Type: application/json' --data-binary "@$payload" "$endpoint" >/dev/null 2>&1
}

flush_events() {
  parse_run_id_args "$@"
  run_id="$parsed_run_id"
  ensure_storage
  outbox="$(outbox_file "$run_id")"
  pending="$(pending_events "$outbox")"
  if [ "${AUTOSPEC_OBSERVATORY_OFFLINE:-}" = "1" ]; then
    update_upload_checkpoint "$run_id" "offline" 0 0
    printf 'STATUS:offline run_id=%s pending_events=%s\n' "$run_id" "$pending"
    return 0
  fi
  url="${AUTOSPEC_OBSERVATORY_URL:-}"
  if [ -z "$url" ]; then
    update_upload_checkpoint "$run_id" "queued" 1 2
    printf 'STATUS:queued run_id=%s pending_events=%s reason=no_url\n' "$run_id" "$pending"
    return 0
  fi
  case "$url" in */v1/events/batch) endpoint="$url" ;; *) endpoint="${url%/}/v1/events/batch" ;; esac
  if [ ! -s "$outbox" ]; then
    update_upload_checkpoint "$run_id" "uploaded" 0 0
    printf 'STATUS:uploaded run_id=%s pending_events=0\n' "$run_id"
    return 0
  fi
  payload="$(mktemp "$(observatory_dir)/events.XXXXXX")"
  jq -s --arg run_id "$run_id" '{run_id:$run_id, events:.}' "$outbox" > "$payload"
  if post_payload "$payload" "$endpoint"; then
    update_upload_checkpoint "$run_id" "uploaded" 0 0
    printf 'STATUS:uploaded run_id=%s pending_events=%s\n' "$run_id" "$pending"
  else
    queue_upload "$run_id" "$pending"
  fi
  rm -f "$payload"
}

status_events() {
  run_id="${AUTOSPEC_RUN_ID:-}"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --run-id) run_id="${2:-}"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) fail_usage "unknown status option: $1" ;;
    esac
  done
  [ -n "$run_id" ] || fail_usage "status requires --run-id"
  ensure_storage
  outbox="$(outbox_file "$run_id")"
  pending=0
  if [ -f "$outbox" ]; then
    pending="$(wc -l < "$outbox" | tr -d ' ')"
  fi
  upload_status="$(checkpoint_value "$run_id" upload_status pending)"
  last_sequence="$(checkpoint_value "$run_id" last_sequence 0)"
  retry_count="$(checkpoint_value "$run_id" retry_count 0)"
  next_retry_at="$(checkpoint_value "$run_id" next_retry_at '')"
  printf 'run_id=%s\n' "$run_id"
  printf 'outbox=%s\n' "$outbox"
  printf 'pending_events=%s\n' "$pending"
  printf 'last_sequence=%s\n' "$last_sequence"
  printf 'upload_status=%s\n' "$upload_status"
  printf 'retry_count=%s\n' "$retry_count"
  printf 'next_retry_at=%s\n' "$next_retry_at"
}

dry_run() {
  args=("$@")
  emit_event "${args[@]}" --event-type RunStarted --status running --summary "autospec run started" >/dev/null
  emit_event "${args[@]}" --event-type WorkerHeartbeat --status running --summary "worker heartbeat" >/dev/null
  flush_events --run-id "$(extract_run_id "${args[@]}")"
}

extract_run_id() {
  run_id="${AUTOSPEC_RUN_ID:-}"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --run-id) run_id="${2:-}"; shift 2 ;;
      *) shift ;;
    esac
  done
  [ -n "$run_id" ] || fail_usage "dry-run requires --run-id"
  printf '%s' "$run_id"
}

main() {
  cmd="${1:-}"
  if [ $# -gt 0 ]; then shift; fi
  case "$cmd" in
    emit) emit_event "$@" ;;
    dry-run) dry_run "$@" ;;
    flush) flush_events "$@" ;;
    status) status_events "$@" ;;
    --help|-h|help) usage ;;
    *) fail_usage "unknown command: ${cmd:-}" ;;
  esac
}

main "$@"
