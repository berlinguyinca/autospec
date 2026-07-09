#!/usr/bin/env bats
# Tests for advisor-report.sh — telemetry summary + promotion-gate arithmetic.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-report.sh"
  TMP="$(mktemp -d)"
  cat > "$TMP/t.jsonl" <<'EOF'
{"ts":"t","issue":"1","repo":"a/b","gate":"impl-haiku","verdict":"plan","tokens_out":100,"over_budget":false}
{"ts":"t","issue":"2","repo":"a/b","gate":"impl-haiku","verdict":"stop","tokens_out":50,"over_budget":false}
{"ts":"t","issue":"3","repo":"a/b","gate":"reviewer","verdict":"plan","tokens_out":80,"over_budget":true}
EOF
}

teardown() { rm -rf "$TMP"; }

@test "report counts calls per gate" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.["impl-haiku"].calls == 2' >/dev/null
  echo "$output" | jq -e '.["reviewer"].calls == 1' >/dev/null
}

@test "report aggregates verdict + token + over_budget counts" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.["impl-haiku"].plan == 1' >/dev/null
  echo "$output" | jq -e '.["impl-haiku"].stop == 1' >/dev/null
  echo "$output" | jq -e '.["impl-haiku"].total_tokens_out == 150' >/dev/null
  echo "$output" | jq -e '.["reviewer"].over_budget == 1' >/dev/null
}

@test "promote true when quality up and cost down" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.72 --baseline-cost 1000 --observed-cost 900 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == true' >/dev/null
}

@test "promote false when cost regressed" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.75 --baseline-cost 1000 --observed-cost 1100 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == false' >/dev/null
}

@test "promote false when quality regressed" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.65 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == false' >/dev/null
}

@test "missing telemetry file exits 1" {
  run bash "$SCRIPT" --telemetry "$TMP/nope.jsonl" --json
  [ "$status" -eq 1 ]
}

@test "non-numeric baseline exits 1 cleanly (no jq stack)" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm abc --observed-lgtm 0.7 --baseline-cost 1000 --observed-cost 900 --json
  [ "$status" -eq 1 ]
  echo "$output" | grep -q 'must be numeric'
}

@test "a malformed telemetry line is skipped, not fatal" {
  printf 'not json at all\n' >> "$TMP/t.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" --json
  [ "$status" -eq 0 ]
  # The three well-formed lines still aggregate.
  echo "$output" | jq -e '.["impl-haiku"].calls == 2' >/dev/null
}
