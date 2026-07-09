#!/usr/bin/env bats
# Tests for advisor-sweep-tick.sh — the end-of-run self-governance orchestrator.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-sweep-tick.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_CONFIG_FILE="$TMP/none.yml"        # hermetic config
  export AUTOSPEC_ADVISOR_POLICY=auto
  export AUTOSPEC_ADVISOR_STATE_DIR="$TMP/state"
  MAIN="$TMP/main.jsonl"
  ADV="$TMP/advisor-escalate.jsonl"
  BASE="$TMP/baseline.json"

  # One reviewer issue, warm-cache first pass → observed lgtm=1.0; tokens control cost.
  mk_main_good() {
    cat > "$MAIN" <<'EOF'
{"ts":"t1","role":"implementer","issue":"1","input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0}
{"ts":"t2","role":"reviewer","issue":"1","input_tokens":20,"output_tokens":10,"cache_read_input_tokens":900}
EOF
  }
  mk_adv_samples() {   # N advisor-call lines for the sample floor
    : > "$ADV"; local i=0
    while [ "$i" -lt "$1" ]; do
      printf '{"gate":"impl-haiku","verdict":"plan"}\n' >> "$ADV"; i=$((i+1))
    done
  }
}

teardown() { rm -rf "$TMP"; }

@test "policy not auto → skip" {
  export AUTOSPEC_ADVISOR_POLICY=on
  mk_main_good
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "skip"' >/dev/null
}

@test "first activation with signal captures the baseline and stops" {
  mk_main_good
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "baseline-captured"' >/dev/null
  [ -f "$BASE" ]
  jq -e '.lgtm_first_pass == 1 and .cost_per_issue == 180' "$BASE" >/dev/null
}

@test "no reviewer signal → hold (no baseline written)" {
  : > "$MAIN"
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
  [ ! -f "$BASE" ]
}

@test "baseline exists + improvement + enough samples → promote" {
  printf '{"lgtm_first_pass":0.5,"cost_per_issue":500}' > "$BASE"
  mk_main_good          # observed lgtm=1.0 >= 0.5, cost=180 <= 500
  mk_adv_samples 25     # above the min-sample floor (20)
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "promote"' >/dev/null
  echo "$output" | jq -e '.active == ["impl-haiku","retry"]' >/dev/null
}

@test "baseline exists + regression → retract (seed preserved)" {
  printf '{"lgtm_first_pass":0.9,"cost_per_issue":100}' > "$BASE"   # observed will look worse
  mk_main_good          # observed lgtm=1.0 (ok) but cost=180 > 100 → regression
  mk_adv_samples 25
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json"
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "retract"' >/dev/null
  echo "$output" | jq -e '.active == ["impl-haiku"]' >/dev/null
}

@test "below the sample floor → hold" {
  printf '{"lgtm_first_pass":0.5,"cost_per_issue":500}' > "$BASE"
  mk_main_good
  mk_adv_samples 3      # below floor
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
}
