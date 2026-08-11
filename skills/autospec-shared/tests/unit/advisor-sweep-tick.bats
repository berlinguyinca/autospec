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
  OUTCOMES="$TMP/review-outcomes.jsonl"

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
  mk_clean_outcomes() {
    : > "$OUTCOMES"; local i=1
    while [ "$i" -le "$1" ]; do
      printf '{"schema":1,"outcome_digest":"sha256:o%d","pr":%d,"commit":"%040d","review_receipt_digest":"sha256:r%d","reviewer_harness":"codex","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run"}\n' "$i" "$i" "$i" "$i" >> "$OUTCOMES"
      i=$((i+1))
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

@test "existing baseline + zero signal → hold, active set unchanged (paramount)" {
  printf '{"lgtm_first_pass":0.9,"cost_per_issue":100}' > "$BASE"
  : > "$MAIN"                       # no reviewer signal this window
  mk_adv_samples 25
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json"
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
  # must NOT retract on absent signal — active set is untouched
  jq -e '.active == ["impl-haiku","retry"]' "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json" >/dev/null
}

@test "below the sample floor → hold" {
  printf '{"lgtm_first_pass":0.5,"cost_per_issue":500}' > "$BASE"
  mk_main_good
  mk_adv_samples 3      # below floor
  run bash "$SCRIPT" --main-telemetry "$MAIN" --advisor-telemetry "$ADV" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
}

@test "literal high escape strengthens immediately without twenty samples" {
  cat > "$OUTCOMES" <<'EOF'
{"schema":1,"outcome_digest":"sha256:o1","pr":1,"commit":"1111111111111111111111111111111111111111","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":1,"escaped_total":1,"review_cost":100,"phase55_run":"run"}
{"schema":1,"outcome_digest":"sha256:o2","pr":2,"commit":"2222222222222222222222222222222222222222","review_receipt_digest":"sha256:r2","reviewer_harness":"codex","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run"}
{"schema":1,"outcome_digest":"sha256:o3","pr":3,"commit":"3333333333333333333333333333333333333333","review_receipt_digest":"sha256:r3","reviewer_harness":"codex","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run"}
{"schema":1,"outcome_digest":"sha256:o4","pr":4,"commit":"4444444444444444444444444444444444444444","review_receipt_digest":"sha256:r4","reviewer_harness":"codex","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run"}
EOF
  printf '{"escaped_high_rate":0,"escaped_total_rate":0.25,"cost_per_reviewed_pr":100}' > "$BASE"
  run bash "$SCRIPT" --review-outcomes "$OUTCOMES" --baseline-file "$BASE" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "strengthen" and .samples == 4' >/dev/null
}

@test "literal nineteen clean outcomes hold relaxation and twenty may relax within cost" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json"
  printf '{"escaped_high_rate":0,"escaped_total_rate":0.1,"cost_per_reviewed_pr":200}' > "$BASE"
  mk_clean_outcomes 19
  run bash "$SCRIPT" --review-outcomes "$OUTCOMES" --baseline-file "$BASE" --min-samples 20 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold" and .samples == 19' >/dev/null
  mk_clean_outcomes 20
  run bash "$SCRIPT" --review-outcomes "$OUTCOMES" --baseline-file "$BASE" --min-samples 20 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "relax" and .samples == 20 and .active == ["impl-haiku"]' >/dev/null
}

@test "literal review_unavailable outcome freezes relaxation" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR"
  printf '{"active":["impl-haiku","retry"]}' > "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json"
  printf '{"escaped_high_rate":0,"escaped_total_rate":0.1,"cost_per_reviewed_pr":200}' > "$BASE"
  mk_clean_outcomes 20
  printf '%s\n' '{"schema":1,"outcome_digest":"sha256:unavailable","outcome":"review_unavailable","pr":null,"phase55_run":"run"}' >> "$OUTCOMES"
  run bash "$SCRIPT" --review-outcomes "$OUTCOMES" --baseline-file "$BASE" --min-samples 20 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold" and .reason == "review_unavailable"' >/dev/null
  jq -e '.active == ["impl-haiku","retry"]' "$AUTOSPEC_ADVISOR_STATE_DIR/active-gates.json" >/dev/null
}
