#!/usr/bin/env bats
# Tests for advisor-govern.sh — the telemetry-driven self-governance ratchet.
#
# Under policy: auto, autospec promotes/retracts the active gate set from its own
# telemetry: a gate is promoted (next in the safety order) only when promote=true
# holds over a minimum-sample floor; it retracts the last-added gate on
# regression. impl-haiku is the seed and is never retracted.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-govern.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_ADVISOR_STATE_DIR="$TMP/state"
  STATE="$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json"
  # A telemetry file with N well-formed lines (verdict irrelevant to sample count).
  mk_telemetry() {
    local n="$1" f="$2"; : > "$f"
    local i=0
    while [ "$i" -lt "$n" ]; do
      printf '{"ts":"t","issue":"%d","repo":"a/b","gate":"impl-haiku","verdict":"plan","tokens_out":10,"use_count":1,"over_budget":false}\n' "$i" >> "$f"
      i=$((i + 1))
    done
  }
}

teardown() { rm -rf "$TMP"; }

quality_tick() {
  bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples "${1}" \
    --attributed-samples "${2}" --escaped-high-rate "${3}" \
    --baseline-total-rate "${4}" --escaped-total-rate "${5}" \
    --baseline-cost "${6}" --observed-cost "${7}" \
    --review-unavailable "${8}" --json
}

@test "seed active set is impl-haiku when no state exists" {
  run bash "$SCRIPT" show --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
}

@test "holds (no promotion) below the min-sample floor" {
  mk_telemetry 5 "$TMP/t.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
  echo "$output" | jq -e '.action == "hold"' >/dev/null
}

@test "promotes the next gate when promote=true above the floor" {
  mk_telemetry 25 "$TMP/t.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku","retry"]' >/dev/null
  echo "$output" | jq -e '.action == "promote"' >/dev/null
}

@test "retracts the last-added gate on regression, never below the seed" {
  mk_telemetry 25 "$TMP/t.jsonl"
  # First promote to [impl-haiku, retry]
  bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  # Now a regression (cost up) → retract back to seed
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 1200 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
  echo "$output" | jq -e '.action == "retract"' >/dev/null
}

@test "retract never drops the impl-haiku seed" {
  mk_telemetry 25 "$TMP/t.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.5 --baseline-cost 1000 --observed-cost 1200 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
}

@test "promotes at most one gate per tick (ratchet)" {
  mk_telemetry 25 "$TMP/t.jsonl"
  bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku","retry","reviewer"]' >/dev/null
}

@test "empty telemetry file holds — never promotes on zero evidence" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/empty.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.9 --baseline-cost 1000 --observed-cost 500 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
  echo "$output" | jq -e '.samples == 0' >/dev/null
}

@test "promotes on an exact tie (quality == baseline AND cost == baseline)" {
  mk_telemetry 25 "$TMP/t.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.7 --baseline-cost 1000 --observed-cost 1000 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "promote"' >/dev/null
}

@test "full active set is stable — a promote tick does not grow past the order" {
  mk_telemetry 25 "$TMP/t.jsonl"
  # Seed the state at the full set.
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry","reviewer","impl-decision"]}' > "$STATE"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 0.7 --observed-lgtm 0.9 --baseline-cost 1000 --observed-cost 500 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku","retry","reviewer","impl-decision"]' >/dev/null
}

@test "corrupted active-gates.json self-heals: unknown names dropped, seed kept, ORDER normalized" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["reviewer","bogus/name","impl-haiku"]}' > "$STATE"
  run bash "$SCRIPT" show --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["impl-haiku","reviewer"]' >/dev/null
}

@test "non-numeric baseline exits 1 (no set -e abort)" {
  mk_telemetry 25 "$TMP/t.jsonl"
  run bash "$SCRIPT" tick --telemetry "$TMP/t.jsonl" --min-samples 20 \
    --baseline-lgtm 1. --observed-lgtm 0.8 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 1 ]
}

@test "an attributed high-severity escape strengthens immediately below the relaxation floor" {
  : > "$TMP/t.jsonl"
  run quality_tick 20 4 0.25 0.25 0.25 100 100 false
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "strengthen" and .active == ["impl-haiku","retry"]' >/dev/null
}

@test "fewer than twenty clean attributed samples holds relaxation" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$STATE"
  : > "$TMP/t.jsonl"
  run quality_tick 20 19 0 0.1 0 200 100 false
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold" and .active == ["impl-haiku","retry"]' >/dev/null
}

@test "twenty clean attributed samples may relax when total escapes and cost do not regress" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$STATE"
  : > "$TMP/t.jsonl"
  run quality_tick 20 20 0 0.1 0 200 100 false
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "relax" and .active == ["impl-haiku"]' >/dev/null
}

@test "review unavailable freezes policy relaxation" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$STATE"
  : > "$TMP/t.jsonl"
  run quality_tick 20 20 0 0.1 0 200 100 true
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold" and .reason == "review_unavailable" and .active == ["impl-haiku","retry"]' >/dev/null
}
