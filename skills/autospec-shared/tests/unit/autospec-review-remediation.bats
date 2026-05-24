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
   "title":"trailing pipe matches every line on BSD grep","body":"build pattern drops trailing \\|","verdict":"keep","dedupe_key":"cross-repo-search-trailing-pipe"},
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
