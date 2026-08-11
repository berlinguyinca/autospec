#!/usr/bin/env bats
# autospec-review-remediation.bats — tests the emit-gaps.sh shaper:
# valid JSON schema emitted; seeded false-positive dropped; seeded broad defect surfaces.

EMIT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/emit-gaps.sh"
LIB="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/gap-json-lib.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    # Candidate findings: one genuine correctness defect (keep), one flagged false-positive (drop).
    cat > "$TEST_TMP/findings.json" <<'EOF'
[
  {"dimension":"correctness","severity":"medium","file":"cross-repo-search.sh","line":77,
   "title":"trailing pipe matches every line on BSD grep","body":"build pattern drops trailing \\|","verdict":"keep","dedupe_key":"cross-repo-search-trailing-pipe",
   "originating_pr":123,"originating_commit":"0123456789abcdef0123456789abcdef01234567",
   "review_receipt_digest":"sha256:receipt","reviewer_harness":"codex","reviewer_reasoning":"high",
   "provider_diversified":true,"review_risk":"integration"},
  {"dimension":"test-quality","severity":"low","file":"x.sh","line":1,
   "title":"phantom defect","body":"reviewer hallucinated","verdict":"false_positive","dedupe_key":"phantom"}
]
EOF
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "emit-gaps.sh is executable" {
    [ -x "$EMIT" ]
}

@test "emits a JSON array where every object satisfies the gap schema" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/gaps.json" ]
    jq -e 'type == "array"' "$TEST_TMP/gaps.json"
    # each element validates against the shared schema
    n="$(jq 'length' "$TEST_TMP/gaps.json")"
    for i in $(seq 0 $((n - 1))); do
        obj="$(jq -c ".[$i]" "$TEST_TMP/gaps.json")"
        run bash -c ". '$LIB'; gap_validate_object '$obj'"
        [ "$status" -eq 0 ]
    done
}

@test "false-positive verdict is dropped by the filter" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq -r '.[].dedupe_key' "$TEST_TMP/gaps.json"
    [[ "$output" == *"cross-repo-search-trailing-pipe"* ]]
    [[ "$output" != *"phantom"* ]]
}

@test "seeded broad-dimension defect (correctness) surfaces as a gap with gap_id" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq -r '.[0].gap_id' "$TEST_TMP/gaps.json"
    [ -n "$output" ]
    run jq -r '.[0].dimension' "$TEST_TMP/gaps.json"
    [ "$output" = "correctness" ]
}

@test "empty findings produce an empty array" {
    printf '[]\n' > "$TEST_TMP/findings.json"
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq 'length' "$TEST_TMP/gaps.json"
    [ "$output" = "0" ]
}

@test "review attribution survives emission and validation" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    jq -e '.[0] | .attribution_status == "attributed" and
      .originating_pr == 123 and
      .originating_commit == "0123456789abcdef0123456789abcdef01234567" and
      .review_receipt_digest == "sha256:receipt" and
      .reviewer_harness == "codex" and .reviewer_reasoning == "high" and
      .provider_diversified == true and .review_risk == "integration"' "$TEST_TMP/gaps.json"
    obj="$(jq -c '.[0]' "$TEST_TMP/gaps.json")"
    run bash -c ". '$LIB'; gap_validate_object '$obj'"
    [ "$status" -eq 0 ]
}

@test "missing attribution is explicit and cannot masquerade as attributed" {
    jq '.[0] |= del(.originating_pr,.originating_commit,.review_receipt_digest,.reviewer_harness,.reviewer_reasoning,.provider_diversified,.review_risk)' \
      "$TEST_TMP/findings.json" > "$TEST_TMP/unattributed.json"
    run bash "$EMIT" --findings "$TEST_TMP/unattributed.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    jq -e '.[0] | .attribution_status == "unavailable" and
      .originating_pr == null and .originating_commit == null and
      .review_receipt_digest == null and .reviewer_harness == null and
      .reviewer_reasoning == null and .provider_diversified == null and
      .review_risk == null' "$TEST_TMP/gaps.json"
}

@test "emission appends an immutable attributed review outcome and correction supersedes it" {
    cat > "$TEST_TMP/review-metadata.json" <<'EOF'
{"pr":123,"commit":"0123456789abcdef0123456789abcdef01234567","review_receipt_digest":"sha256:receipt","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"review_cost":400,"phase55_run":"run-1"}
EOF
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json" \
      --review-metadata "$TEST_TMP/review-metadata.json" --outcomes "$TEST_TMP/review-outcomes.jsonl"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$TEST_TMP/review-outcomes.jsonl")" -eq 1 ]
    jq -e '.schema == 1 and .pr == 123 and .escaped_high_severity == 0 and
      .escaped_total == 1 and .review_cost == 400 and
      (.outcome_digest | startswith("sha256:"))' "$TEST_TMP/review-outcomes.jsonl"
    first_digest="$(jq -r '.outcome_digest' "$TEST_TMP/review-outcomes.jsonl")"
    jq --arg supersedes "$first_digest" '.phase55_run = "run-1-correction" |
      .supersedes_outcome_digest = $supersedes' "$TEST_TMP/review-metadata.json" > "$TEST_TMP/correction.json"
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps-correction.json" \
      --review-metadata "$TEST_TMP/correction.json" --outcomes "$TEST_TMP/review-outcomes.jsonl"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$TEST_TMP/review-outcomes.jsonl")" -eq 2 ]
    jq -s -e --arg supersedes "$first_digest" '.[1].supersedes_outcome_digest == $supersedes and
      .[1].outcome_digest != .[0].outcome_digest' "$TEST_TMP/review-outcomes.jsonl"
}

@test "failed broad review emits review_unavailable instead of an empty clean result" {
    run bash "$EMIT" --findings "$TEST_TMP/missing.json" --out "$TEST_TMP/gaps.json" \
      --outcomes "$TEST_TMP/review-outcomes.jsonl" --review-unavailable --phase55-run run-failed
    [ "$status" -eq 0 ]
    jq -e 'length == 1 and .[0].outcome == "review_unavailable" and
      .[0].attribution_status == "unavailable"' "$TEST_TMP/gaps.json"
    jq -e '.outcome == "review_unavailable" and .phase55_run == "run-failed" and
      (.outcome_digest | startswith("sha256:"))' "$TEST_TMP/review-outcomes.jsonl"
}
