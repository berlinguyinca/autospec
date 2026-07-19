#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/dogfood-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  export AUTOSPEC_OBSERVATORY_DIR="$TEST_TMP/.autospec/observatory"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "offline dogfood writes timeline and cost artifacts without external services" {
  run bash "$SCRIPT" --offline --run-id dogfood-test --output-dir "$TEST_TMP/artifacts"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq 'STATUS:offline'
  printf '%s\n' "$output" | grep -Fq 'timeline_artifact='
  printf '%s\n' "$output" | grep -Fq 'cost_artifact='

  [ -s "$TEST_TMP/artifacts/timeline.json" ]
  [ -s "$TEST_TMP/artifacts/cost-report.json" ]
  [ -s "$TEST_TMP/artifacts/companion-bootstrap.txt" ]
  [ -s "$TEST_TMP/artifacts/replay.log" ]

  jq -e '.run_id == "dogfood-test" and (.events | length) >= 5 and any(.events[]; .event_type == "CostReported")' \
    "$TEST_TMP/artifacts/timeline.json" >/dev/null
  jq -e '.run_id == "dogfood-test" and .total_events >= 5 and .estimated_cost_usd >= 0' \
    "$TEST_TMP/artifacts/cost-report.json" >/dev/null
  grep -Fq 'autospec-governance/' "$TEST_TMP/artifacts/companion-bootstrap.txt"
  grep -Fq 'autospec-observatory/' "$TEST_TMP/artifacts/companion-bootstrap.txt"
}

@test "replay-only mode reuses an existing outbox and refreshes reports" {
  bash "$SCRIPT" --offline --run-id replay-test --output-dir "$TEST_TMP/artifacts" >/dev/null
  rm -f "$TEST_TMP/artifacts/timeline.json" "$TEST_TMP/artifacts/cost-report.json"

  run bash "$SCRIPT" --offline --replay-only --run-id replay-test --output-dir "$TEST_TMP/artifacts"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq 'replay_mode=replay-only'
  [ -s "$TEST_TMP/artifacts/timeline.json" ]
  [ -s "$TEST_TMP/artifacts/cost-report.json" ]
  jq -e '.run_id == "replay-test" and .replay_mode == "replay-only"' "$TEST_TMP/artifacts/timeline.json" >/dev/null
}

@test "help names offline replay and artifacts" {
  run bash "$SCRIPT" --help
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq -- '--offline'
  printf '%s\n' "$output" | grep -Fq -- '--replay-only'
  printf '%s\n' "$output" | grep -Fq 'timeline.json'
  printf '%s\n' "$output" | grep -Fq 'cost-report.json'
}
@test "runbook names timeline and cost artifacts" {
  doc="$REPO_ROOT/docs/runbooks/CONTROL_PLANE_DOGFOOD.md"
  [ -s "$doc" ]
  grep -Fq 'timeline.json' "$doc"
  grep -Fq 'cost-report.json' "$doc"
  grep -Fq 'offline replay' "$doc"
}
