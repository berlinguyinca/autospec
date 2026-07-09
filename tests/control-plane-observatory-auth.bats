#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/observatory-auth-dry-run.txt"
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

@test "dry-run emits tenant API-key model and scope definitions" {
  assert_contains "--- autospec-observatory/apps/api/src/auth/api-keys.ts ---"
  assert_contains "owner_org_id"
  assert_contains "allowed_project_ids"
  assert_contains "allowed_repo_patterns"
  assert_contains "allowed_event_scopes"
  assert_contains "privacy_tier_limit"
  assert_contains "events:write"
  assert_contains "admin:keys"
  assert_contains "Auth failures are written as security events"
}

@test "dry-run emits tenant database migrations for core observatory entities" {
  assert_contains "--- autospec-observatory/migrations/001_create_orgs.sql ---"
  assert_contains "CREATE TABLE orgs"
  assert_contains "--- autospec-observatory/migrations/002_create_projects.sql ---"
  assert_contains "CREATE TABLE projects"
  assert_contains "--- autospec-observatory/migrations/003_create_runs.sql ---"
  assert_contains "CREATE TABLE runs"
  assert_contains "--- autospec-observatory/migrations/004_create_events.sql ---"
  assert_contains "CREATE TABLE events"
}

@test "dry-run route list includes scoped run progress snapshot contract" {
  assert_contains "GET /v1/runs/:id/progress"
  assert_contains "progress_percent"
  assert_contains "phase"
  assert_contains "current_item"
  assert_contains "queue_ready"
  assert_contains "queue_claimed"
  assert_contains "queue_blocked"
  assert_contains "queue_remaining"
  assert_contains "elapsed_ms"
  assert_contains "eta_ms"
  assert_contains "planned_next_step"
  assert_contains "last_event_id"
  assert_contains "Progress reads require runs:read and enforce owner_org_id/project boundaries"
}
