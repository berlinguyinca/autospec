#!/usr/bin/env bats
# Tests for grooming-observe.sh — derives the groomed vs baseline clean-merge
# rate from autospec's telemetry JSONL, feeding grooming-govern.sh's tick.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/grooming-observe.sh"
  TMP="$(mktemp -d)"
  T="$TMP/telemetry.jsonl"
  # 2 groomed issues (1 clean, 1 escalated) + 2 ungroomed issues (both clean).
  cat > "$T" <<'EOF'
{"issue":"1","groomed":true,"reverted":false,"reopened":false,"labels":[]}
{"issue":"2","groomed":true,"reverted":false,"reopened":false,"labels":["escalate:human"]}
{"issue":"3","groomed":false,"reverted":false,"reopened":false,"labels":[]}
{"issue":"4","groomed":false,"reverted":false,"reopened":false,"labels":[]}
EOF
}

teardown() { rm -rf "$TMP"; }

@test "computes groomed_clean_merge_rate and samples from groomed issues" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.groomed_clean_merge_rate == 0.5' >/dev/null
  echo "$output" | jq -e '.samples == 2' >/dev/null
}

@test "computes baseline_clean_merge_rate from ungroomed issues" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_clean_merge_rate == 1' >/dev/null
}

@test "emits baseline_samples counting ungroomed records (widen-guard input)" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_samples == 2' >/dev/null
}

@test "zeroed metrics include baseline_samples:0" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/empty.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_samples == 0' >/dev/null
}

@test "empty/missing telemetry yields zeroed metrics, exit 0 (fail-safe)" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/empty.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.groomed_clean_merge_rate == 0 and .baseline_clean_merge_rate == 0 and .samples == 0' >/dev/null
}

@test "malformed telemetry lines are skipped, not fatal" {
  printf 'garbage not json\n' >> "$T"
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.samples == 2' >/dev/null
}

@test "nonexistent telemetry path yields zeroed metrics, exit 0" {
  run bash "$SCRIPT" --telemetry "$TMP/nope.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.samples == 0' >/dev/null
}
