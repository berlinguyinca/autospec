#!/usr/bin/env bats
# tests/qa/test_qa_heal_default.bats — /autospec-qa heal-default-on contract
# (issue #711). Locks in:
#   1. Heal loop runs by default (no flags → loop runs)
#   2. --single-pass alias works as opt-out
#   3. ~/.autospec/qa-no-heal.flag opt-out works
#   4. ~/.autospec/no-heal.flag legacy opt-out still works
#   5. ~/.autospec/qa-heal-stop.flag per-skill stop sentinel
#   6. STOP marker in verdict triggers evidence_based_stop
#   7. .autospec/qa-heal-summary.md is mirrored from heal-summary.md
#   8. Shared loop driver scripts/lib/autospec-loop.sh is sourceable

setup() {
    HEAL_SH="$BATS_TEST_DIRNAME/../../scripts/qa-heal-loop.sh"
    LOOP_LIB="$BATS_TEST_DIRNAME/../../scripts/lib/autospec-loop.sh"
    [ -x "$HEAL_SH" ] || skip "qa-heal-loop.sh not installed"
    command -v jq >/dev/null 2>&1 || skip "jq required"
    TMP="$(mktemp -d)"
    FIX="$TMP/fixtures"
    mkdir -p "$FIX"
    VERDICT="$TMP/.autospec/qa-verdict.json"
    SUMMARY="$TMP/.autospec/heal-summary.md"
    mkdir -p "$TMP/.autospec"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    export AUTOSPEC_RUN_CMD="echo 0"
}

teardown() {
    [ -n "${TMP:-}" ] && rm -rf "$TMP"
    return 0
}

mk_round() {
    local n="$1" count="$2" tag="${3:-default}"
    local findings="[]"
    if [ "$count" -gt 0 ]; then
        findings=$(jq -nc --arg tag "$tag" --argjson n "$count" '
            [range(0; $n) | {
                category: "no_mock_smoke",
                release_blocking: true,
                status: "FAIL",
                summary: ("finding-" + ($tag) + "-" + (. | tostring)),
                file: ("src/file-" + (. | tostring) + ".ts"),
                evidence: "tests/x.spec.ts"
            }]
        ')
    fi
    local verdict="PARTIAL"
    [ "$count" = 0 ] && verdict="PASS"
    jq -n --arg v "$verdict" --argjson f "$findings" \
        '{verdict: $v, head_sha: "abc1234", findings: $f}' \
        > "$FIX/round-${n}.json"
}

@test "default invocation (no flags) loops until convergence — heal is default-on" {
    mk_round 1 2 d1
    mk_round 2 0
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"convergence"* ]]
    # Two rounds → multi-round behavior proves heal was NOT skipped.
    grep -q "^| 1 " "$SUMMARY"
    grep -q "^| 2 " "$SUMMARY"
}

@test "--single-pass alias runs once like --no-heal" {
    mk_round 1 4 sp
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --single-pass
    [ "$status" -eq 0 ]
    [[ "$output" == *"no_heal"* ]]
    grep -q "no_heal" "$SUMMARY"
}

@test "~/.autospec/qa-no-heal.flag forces single-pass" {
    mk_round 1 3 qnh
    touch "$HOME/.autospec/qa-no-heal.flag"
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no_heal"* ]]
}

@test "~/.autospec/no-heal.flag (legacy) still forces single-pass" {
    mk_round 1 3 lnh
    touch "$HOME/.autospec/no-heal.flag"
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no_heal"* ]]
}

@test "~/.autospec/qa-heal-stop.flag stops loop at round boundary" {
    mk_round 1 3 stop1
    mk_round 2 3 stop2
    touch "$HOME/.autospec/qa-heal-stop.flag"
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --max-rounds 5
    [ "$status" -eq 1 ]
    [[ "$output" == *"stopped"* ]]
}

@test "stop_marker in verdict triggers evidence_based_stop" {
    # Construct a round-1 fixture with a stop_marker field.
    jq -n '{
        verdict: "PARTIAL",
        head_sha: "abc1234",
        stop_marker: "operator-directive: pause for review",
        findings: [{
            category: "no_mock_smoke",
            release_blocking: true,
            status: "FAIL",
            summary: "finding-evb-0",
            file: "src/a.ts",
            evidence: "t.spec.ts"
        }]
    }' > "$FIX/round-1.json"
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --max-rounds 5
    [ "$status" -eq 1 ]
    [[ "$output" == *"evidence_based_stop"* ]]
    grep -q "evidence_based_stop" "$SUMMARY"
}

@test "qa-heal-summary.md mirror is written alongside heal-summary.md" {
    mk_round 1 0
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [ -f "$SUMMARY" ]
    [ -f "$TMP/.autospec/qa-heal-summary.md" ]
    # Mirror content equals primary summary.
    diff -q "$SUMMARY" "$TMP/.autospec/qa-heal-summary.md"
}

@test "shared loop driver scripts/lib/autospec-loop.sh is sourceable" {
    [ -f "$LOOP_LIB" ] || skip "shared loop driver not installed"
    run bash -c ". '$LOOP_LIB' && declare -F autospec_loop_run"
    [ "$status" -eq 0 ]
    [[ "$output" == *"autospec_loop_run"* ]]
}
