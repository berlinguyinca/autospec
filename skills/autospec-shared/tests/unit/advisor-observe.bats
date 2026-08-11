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

write_outcomes() {
  cat > "$TMP/outcomes.jsonl" <<'EOF'
{"schema":1,"outcome_digest":"sha256:o1","pr":1,"commit":"1111111111111111111111111111111111111111","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":1,"escaped_total":1,"review_cost":100,"cache_read_input_tokens":999999,"phase55_run":"run-1"}
{"schema":1,"outcome_digest":"sha256:o2","pr":2,"commit":"2222222222222222222222222222222222222222","review_receipt_digest":"sha256:r2","reviewer_harness":"claude","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"cache_read_input_tokens":0,"phase55_run":"run-1"}
{"schema":1,"outcome_digest":"sha256:o3","pr":3,"commit":"3333333333333333333333333333333333333333","review_receipt_digest":"sha256:r3","reviewer_harness":"opencode","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"high","first_pass_lgtm":false,"escaped_high_severity":0,"escaped_total":1,"review_cost":100,"cache_read_input_tokens":0,"phase55_run":"run-1"}
{"schema":1,"outcome_digest":"sha256:o4","pr":4,"commit":"4444444444444444444444444444444444444444","review_receipt_digest":"sha256:r4","reviewer_harness":"codex","reviewer_reasoning":"standard","provider_diversified":false,"review_risk":"normal","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"cache_read_input_tokens":0,"phase55_run":"run-1"}
EOF
}

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

@test "one high escape across four attributed reviewed PRs is exactly 0.25" {
  write_outcomes
  run bash "$SCRIPT" --outcomes "$TMP/outcomes.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.escaped_high_rate == 0.25 and .attributed_reviewed_prs == 4' >/dev/null
  echo "$output" | jq -e '.escaped_total_rate == 0.5 and .cost_per_reviewed_pr == 100' >/dev/null
}

@test "cache tokens are irrelevant to quality and first-pass LGTM is diagnostic only" {
  write_outcomes
  run bash "$SCRIPT" --outcomes "$TMP/outcomes.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.escaped_high_rate == 0.25 and .first_pass_lgtm == 0.75' >/dev/null
  jq -c '.cache_read_input_tokens = 0' "$TMP/outcomes.jsonl" > "$TMP/outcomes.rewritten.jsonl"
  mv "$TMP/outcomes.rewritten.jsonl" "$TMP/outcomes.jsonl"
  run bash "$SCRIPT" --outcomes "$TMP/outcomes.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.escaped_high_rate == 0.25 and .escaped_total_rate == 0.5' >/dev/null
}

@test "superseding correction replaces the prior observation without rewriting history" {
  write_outcomes
  cat >> "$TMP/outcomes.jsonl" <<'EOF'
{"schema":1,"outcome_digest":"sha256:o1-correction","supersedes_outcome_digest":"sha256:o1","pr":1,"commit":"1111111111111111111111111111111111111111","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run-1-correction"}
EOF
  run bash "$SCRIPT" --outcomes "$TMP/outcomes.jsonl" --json
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$TMP/outcomes.jsonl")" -eq 5 ]
  echo "$output" | jq -e '.attributed_reviewed_prs == 4 and .escaped_high_rate == 0 and .escaped_total_rate == 0.25' >/dev/null
}

@test "unattributed rows remain explicit and never count as clean reviewed samples" {
  printf '%s\n' '{"schema":1,"outcome_digest":"sha256:u1","outcome":"unattributed","pr":null,"escaped_high_severity":0,"escaped_total":0,"phase55_run":"run-u"}' > "$TMP/outcomes.jsonl"
  run bash "$SCRIPT" --outcomes "$TMP/outcomes.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.attributed_reviewed_prs == 0 and .review_unavailable == false' >/dev/null
}
