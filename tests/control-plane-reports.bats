#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/control-plane-reports-dry-run.txt"
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

@test "dry-run renders seven MVP cost duration outcome report names" {
  assert_contains "Project weekly summary"
  assert_contains "Client billing export"
  assert_contains "Open-source maintenance report"
  assert_contains "Agent performance report"
  assert_contains "Cost anomaly report"
  assert_contains "Blocked work report"
  assert_contains "Autonomous ROI report"
}

@test "dry-run renders report API routes and metric fields" {
  assert_contains "--- autospec-observatory/apps/api/src/reports.ts ---"
  assert_contains "GET /v1/reports/project-weekly-summary"
  assert_contains "GET /v1/reports/client-billing-export"
  assert_contains "GET /v1/reports/autonomous-roi-report"
  assert_contains "estimated_cost_usd"
  assert_contains "duration_ms"
  assert_contains "status_outcome"
  assert_contains "blocked_time_ms"
}

@test "dry-run renders report UI cards with required filters" {
  assert_contains "--- autospec-observatory/apps/web/src/ReportFilters.tsx ---"
  assert_contains "Cost / Duration / Outcome Reports"
  assert_contains "date range"
  assert_contains "privacy tier"
  assert_contains "project classification"
  assert_contains "status/outcome"
  assert_contains "cost range"
  assert_contains "duration range"
}
