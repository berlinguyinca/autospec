#!/usr/bin/env bats
# tests/telemetry-summary.bats — TDD for skills/autospec-shared/scripts/telemetry-summary.sh (issue #403)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/telemetry-summary.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/telemetry-summary"

setup() {
  mkdir -p "$FIXTURES_DIR"
  FIXTURE_JSONL="$FIXTURES_DIR/telemetry.jsonl"
  export AUTOSPEC_TELEMETRY_FILE="$FIXTURE_JSONL"

  # 3-row fixture: 1 miss, 2 hits
  # Row 1: implementer, cache_read=0    (miss)
  # Row 2: implementer, cache_read=5000 (hit)
  # Row 3: reviewer,    cache_read=4800 (hit)
  printf '%s\n' \
    '{"ts":"2026-05-22T10:00:00Z","role":"implementer","issue":403,"input_tokens":12000,"cache_creation_input_tokens":8000,"cache_read_input_tokens":0,"output_tokens":3000,"dispatch_id":"d1"}' \
    '{"ts":"2026-05-22T10:05:00Z","role":"implementer","issue":403,"input_tokens":12000,"cache_creation_input_tokens":0,"cache_read_input_tokens":5000,"output_tokens":2800,"dispatch_id":"d2"}' \
    '{"ts":"2026-05-22T10:10:00Z","role":"reviewer","issue":403,"input_tokens":11000,"cache_creation_input_tokens":0,"cache_read_input_tokens":4800,"output_tokens":1500,"dispatch_id":"d3"}' \
    > "$FIXTURE_JSONL"
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

@test "telemetry-summary.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "telemetry-summary.sh exits 0 on valid JSONL" {
  run "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "telemetry-summary.sh overall cache-hit rate is 2/3 (66%)" {
  run "$SCRIPT"
  [ "$status" -eq 0 ]
  # 2 rows have cache_read_input_tokens > 0 out of 3 → 66%
  printf '%s\n' "$output" | grep -qE '6[67](\.[0-9]+)?%'
}

@test "telemetry-summary.sh shows total dispatches" {
  run "$SCRIPT"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qE '(total|dispatches|Total|Dispatches).*3|3.*(total|dispatches|Total|Dispatches)'
}

@test "telemetry-summary.sh shows per-role breakdown" {
  run "$SCRIPT"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qi "implementer"
  printf '%s\n' "$output" | grep -qi "reviewer"
}

@test "telemetry-summary.sh --role implementer shows only implementer rows" {
  run "$SCRIPT" --role implementer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qi "implementer"
  # Should not show reviewer in the role-filtered output
  ! printf '%s\n' "$output" | grep -qi "^reviewer"
}

@test "telemetry-summary.sh --since filters out old rows" {
  # Add an old row to the fixture
  printf '%s\n' \
    '{"ts":"2026-01-01T00:00:00Z","role":"implementer","issue":100,"input_tokens":5000,"cache_creation_input_tokens":3000,"cache_read_input_tokens":0,"output_tokens":1000,"dispatch_id":"old"}' \
    >> "$FIXTURE_JSONL"
  # Filter to only rows from 2026-05 onwards
  run "$SCRIPT" --since "2026-05-01T00:00:00Z"
  [ "$status" -eq 0 ]
  # Should still show 3 dispatches (original fixture) not 4
  printf '%s\n' "$output" | grep -qE '(total|dispatches|Total|Dispatches).*3|3.*(total|dispatches|Total|Dispatches)'
}

@test "telemetry-summary.sh on empty JSONL exits 0 and reports 0 dispatches" {
  printf '' > "$FIXTURE_JSONL"
  run "$SCRIPT"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qE '0'
}

@test "telemetry-summary.sh exits non-zero when JSONL file missing" {
  export AUTOSPEC_TELEMETRY_FILE="$FIXTURES_DIR/nonexistent.jsonl"
  run "$SCRIPT"
  [ "$status" -ne 0 ]
}
