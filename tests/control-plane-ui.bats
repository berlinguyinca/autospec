#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/control-plane-ui-dry-run.txt"
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

assert_not_contains() {
  local needle="$1"
  if grep -Fq -- "$needle" "$OUTPUT"; then
    printf 'unexpected text present: %s\n' "$needle" >&2
    printf '%s\n' '--- dry-run output ---' >&2
    cat "$OUTPUT" >&2
    return 1
  fi
}

@test "dry-run renders observatory operator UI shell screens with 10-second polling" {
  assert_contains "--- autospec-observatory/apps/web/src/App.tsx ---"
  assert_contains "Live Fleet"
  assert_contains "Run Timeline"
  assert_contains "Work Item Detail"
  assert_contains "Failures / Blockers"
  assert_contains "Workers / Agents"
  assert_contains "Policy Decision Inspector"
  assert_contains "poll_after_ms"
  assert_contains "10000"
  assert_not_contains "WebSocket"
  assert_not_contains "EventSource"
}

@test "dry-run renders per-run progress UI with stale and error state" {
  assert_contains "Run Progress"
  assert_contains "progress_percent"
  assert_contains "Current phase"
  assert_contains "Current item"
  assert_contains "Item elapsed time"
  assert_contains "Queue counts"
  assert_contains "ETA"
  assert_contains "Planned next step"
  assert_contains "Stale heartbeat warning"
  assert_contains "stale/error state"
  assert_contains "progressbar"
}
