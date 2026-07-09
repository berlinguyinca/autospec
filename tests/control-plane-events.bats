#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/control-plane-events-dry-run.txt"
  bash "$SCRIPT" bootstrap --dry-run \
    --owner berlinguyinca \
    --governance-repo autospec-governance \
    --observatory-repo autospec-observatory > "$OUTPUT"
}

teardown() {
  rm -rf "$TEST_TMP"
}

assert_contains() {
  local needle="$1"
  grep -Fq -- "$needle" "$OUTPUT" || {
    printf 'missing expected text: %s\n' "$needle" >&2
    printf '%s\n' '--- dry-run output ---' >&2
    cat "$OUTPUT" >&2
    return 1
  }
}

@test "dry-run emits observatory event ingestion route handlers" {
  assert_contains "--- autospec-observatory/apps/api/src/routes.ts ---"
  assert_contains "POST /v1/events"
  assert_contains "POST /v1/events/batch"
  assert_contains "handleEventIngest"
  assert_contains "handleEventBatchIngest"
}

@test "dry-run emits observatory event schema fields and ProgressUpdated payload fields" {
  assert_contains "--- autospec-observatory/packages/event-schema/src/events.ts ---"
  assert_contains "event_id"
  assert_contains "run_id"
  assert_contains "sequence"
  assert_contains "ProgressUpdated"
  assert_contains "progress_percent"
  assert_contains "progress_phase"
  assert_contains "current_item_title"
  assert_contains "current_item_url"
  assert_contains "queue_ready_count"
  assert_contains "queue_blocked_count"
  assert_contains "queue_claimed_count"
  assert_contains "queue_remaining_count"
  assert_contains "estimated_next_item_at"
  assert_contains "estimated_completion_at"
  assert_contains "planned_next_step"
}

@test "dry-run documents event dedupe and per-run sequence ordering contract" {
  assert_contains "Duplicate event_id is ignored"
  assert_contains "sequence is monotonic per run_id"
  assert_contains "ProgressUpdated follows the same event_id dedupe and per-run sequence ordering contract"
  assert_contains "Sequence gaps are exposed for UI review"
  assert_contains "Late events are stored by occurred_at and received_at"
}
