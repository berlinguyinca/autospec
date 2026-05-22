#!/usr/bin/env bats
# tests/gen-telemetry-dashboard.bats — TDD for skills/autospec-shared/scripts/gen-telemetry-dashboard.sh (issue #422)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/gen-telemetry-dashboard.sh"
FIXTURE="${BATS_TEST_DIRNAME}/fixtures/telemetry/dashboard-input.jsonl"

setup() {
  OUTPUT_FILE=$(mktemp -t telemetry-dashboard-XXXXXX.html)
}

teardown() {
  rm -f "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "gen-telemetry-dashboard.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "gen-telemetry-dashboard.sh exits 1 without --input" {
  run "$SCRIPT" --output "$OUTPUT_FILE"
  [ "$status" -ne 0 ]
}

@test "gen-telemetry-dashboard.sh exits 1 if --input file does not exist" {
  run "$SCRIPT" --input "/nonexistent/path.jsonl" --output "$OUTPUT_FILE"
  [ "$status" -ne 0 ]
}

@test "gen-telemetry-dashboard.sh exits 0 with populated fixture" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
}

@test "gen-telemetry-dashboard.sh output contains cache-hit-rate canvas element" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -q 'cache-hit-rate' "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh output contains per-role token-cost section" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -qi "implementer" "$OUTPUT_FILE"
  grep -qi "reviewer" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh output contains LGTM first-pass rate" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -qi "lgtm\|first.pass\|first-pass" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh output contains top-10 cost outliers table" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -qi "outlier\|cost.*table\|top.*10\|top-10" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh output is valid HTML (has html and body tags)" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -q "<html" "$OUTPUT_FILE"
  grep -q "<body" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh output includes Chart.js CDN link" {
  run "$SCRIPT" --input "$FIXTURE" --output "$OUTPUT_FILE"
  [ "$status" -eq 0 ]
  grep -qi "chart.js\|chartjs\|cdn" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh empty input produces non-empty empty-state HTML" {
  empty_file=$(mktemp -t telemetry-empty-XXXXXX.jsonl)
  run "$SCRIPT" --input "$empty_file" --output "$OUTPUT_FILE"
  rm -f "$empty_file"
  [ "$status" -eq 0 ]
  [ -s "$OUTPUT_FILE" ]
  grep -q "<html" "$OUTPUT_FILE"
}

@test "gen-telemetry-dashboard.sh writes output to stdout when --output is -" {
  run "$SCRIPT" --input "$FIXTURE" --output -
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "<html"
}
