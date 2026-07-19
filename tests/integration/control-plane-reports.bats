#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/control-plane-reports-integration.txt"
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
    cat "$OUTPUT" >&2
    return 1
  }
}

@test "dry-run integrates report API contracts and UI filters" {
  assert_contains "--- autospec-observatory/apps/api/src/reports.ts ---"
  assert_contains "Client billing export"
  assert_contains "estimated_cost_usd"
  assert_contains "duration_ms"
  assert_contains "--- autospec-observatory/apps/web/src/ReportFilters.tsx ---"
  assert_contains "date range"
  assert_contains "privacy tier"
}
