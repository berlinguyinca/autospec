#!/usr/bin/env bats
# Tests for advisor-observe.sh — derives observed LGTM-first-pass rate + mean
# cost/issue from autospec's main telemetry JSONL (same formulas as the dashboard).

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-observe.sh"
  TMP="$(mktemp -d)"
  T="$TMP/telemetry.jsonl"
  # 2 reviewer issues: #1 first-pass (cache warm), #2 not. Costs differ per issue.
  cat > "$T" <<'EOF'
{"ts":"2026-07-01T00:00:00Z","role":"implementer","issue":"1","input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0}
{"ts":"2026-07-01T00:01:00Z","role":"reviewer","issue":"1","input_tokens":20,"output_tokens":10,"cache_read_input_tokens":500}
{"ts":"2026-07-01T00:00:00Z","role":"implementer","issue":"2","input_tokens":300,"output_tokens":150,"cache_read_input_tokens":0}
{"ts":"2026-07-01T00:01:00Z","role":"reviewer","issue":"2","input_tokens":40,"output_tokens":20,"cache_read_input_tokens":0}
EOF
}

teardown() { rm -rf "$TMP"; }

@test "computes lgtm_first_pass as a 0..1 fraction" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  # issue 1 first-pass, issue 2 not → 1/2 = 0.5
  echo "$output" | jq -e '.lgtm_first_pass == 0.5' >/dev/null
  echo "$output" | jq -e '.reviewer_issues == 2' >/dev/null
}

@test "computes cost_per_issue as mean total tokens per issue" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  # issue1 = 100+50+20+10 = 180; issue2 = 300+150+40+20 = 510; mean = 345
  echo "$output" | jq -e '.cost_per_issue == 345' >/dev/null
  echo "$output" | jq -e '.issues == 2' >/dev/null
}

@test "empty/missing telemetry yields zeroed metrics, exit 0 (fail-safe)" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/empty.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lgtm_first_pass == 0 and .cost_per_issue == 0 and .issues == 0' >/dev/null
}

@test "malformed telemetry lines are skipped, not fatal" {
  printf 'garbage not json\n' >> "$T"
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.reviewer_issues == 2' >/dev/null
}

@test "nonexistent telemetry path yields zeroed metrics, exit 0" {
  run bash "$SCRIPT" --telemetry "$TMP/nope.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.issues == 0' >/dev/null
}
