#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-observatory-events.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  export AUTOSPEC_OBSERVATORY_DIR="$TEST_TMP/.autospec/observatory"
  export AUTOSPEC_RUN_ID="run-test-1618"
  export AUTOSPEC_WORKER_ID="worker-test-1618"
}

teardown() {
  rm -rf "$TEST_TMP"
}

jsonl_path() {
  printf '%s/outbox/%s.jsonl' "$AUTOSPEC_OBSERVATORY_DIR" "$AUTOSPEC_RUN_ID"
}

@test "offline smoke creates outbox and serializes run and heartbeat sequences" {
  run env AUTOSPEC_OBSERVATORY_OFFLINE=1 bash "$SCRIPT" dry-run \
    --run-id "$AUTOSPEC_RUN_ID" \
    --worker-id "$AUTOSPEC_WORKER_ID" \
    --repository-id "berlinguyinca/autospec" \
    --issue-url "https://github.com/berlinguyinca/autospec/issues/1618"

  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq 'STATUS:offline'
  [ -d "$AUTOSPEC_OBSERVATORY_DIR/outbox" ]
  [ -s "$(jsonl_path)" ]
  [ "$(wc -l < "$(jsonl_path)" | tr -d ' ')" -eq 2 ]

  jq -e 'select(.event_type == "RunStarted" and .sequence == 1 and .run_id == "run-test-1618")' "$(jsonl_path)" >/dev/null
  jq -e 'select(.event_type == "WorkerHeartbeat" and .sequence == 2 and .worker_id == "worker-test-1618")' "$(jsonl_path)" >/dev/null
  jq -e 'select(.issue_url == "https://github.com/berlinguyinca/autospec/issues/1618")' "$(jsonl_path)" >/dev/null
}

@test "duplicate event ids are skipped and checkpoint records next sequence" {
  run bash "$SCRIPT" emit \
    --run-id "$AUTOSPEC_RUN_ID" \
    --event-type RunStarted \
    --event-id fixed-event-id \
    --summary "first"
  [ "$status" -eq 0 ]

  run bash "$SCRIPT" emit \
    --run-id "$AUTOSPEC_RUN_ID" \
    --event-type RunStarted \
    --event-id fixed-event-id \
    --summary "duplicate"
  [ "$status" -eq 0 ]

  [ "$(wc -l < "$(jsonl_path)" | tr -d ' ')" -eq 1 ]
  jq -e 'select(.summary == "first" and .sequence == 1)' "$(jsonl_path)" >/dev/null
  jq -e --arg run "$AUTOSPEC_RUN_ID" '.[$run].last_sequence == 1 and .[$run].next_sequence == 2' \
    "$AUTOSPEC_OBSERVATORY_DIR/checkpoints.json" >/dev/null
}

@test "flush is offline safe and surfaces retry checkpoint state" {
  bash "$SCRIPT" emit --run-id "$AUTOSPEC_RUN_ID" --event-type RunStarted --summary "queued" >/dev/null

  run env AUTOSPEC_OBSERVATORY_OFFLINE=0 AUTOSPEC_OBSERVATORY_URL="http://127.0.0.1:9" bash "$SCRIPT" flush --run-id "$AUTOSPEC_RUN_ID"

  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq 'STATUS:queued'
  jq -e --arg run "$AUTOSPEC_RUN_ID" '.[$run].upload_status == "queued" and .[$run].retry_count >= 1 and (.[$run].next_retry_at | length > 0)' \
    "$AUTOSPEC_OBSERVATORY_DIR/checkpoints.json" >/dev/null

  run bash "$SCRIPT" status --run-id "$AUTOSPEC_RUN_ID"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq 'upload_status=queued'
  printf '%s\n' "$output" | grep -Fq 'pending_events=1'
}
