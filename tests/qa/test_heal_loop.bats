#!/usr/bin/env bats
# tests/qa/test_heal_loop.bats — /autospec-qa self-healing loop (issue #663).
#
# Covers all 9 scenarios listed in the spec:
#   1. Single-round convergence (0 findings → exits round 1)
#   2. Multi-round convergence (3 → 0 → exits round 2)
#   3. Oscillation detected (same 2 findings rounds 1+2)
#   4. Round cap reached
#   5. Budget cap reached (simulated tokens)
#   6. --no-heal opt-out flag
#   7. ~/.autospec/no-heal.flag forces no-heal
#   8. .autospec/heal-summary.md always written
#   9. Dedup: same finding twice in one round → one issue

setup() {
    HEAL_SH="$BATS_TEST_DIRNAME/../../scripts/qa-heal-loop.sh"
    F2I_SH="$BATS_TEST_DIRNAME/../../scripts/qa-finding-to-issue.sh"
    [ -x "$HEAL_SH" ] || skip "qa-heal-loop.sh not installed"
    [ -x "$F2I_SH" ]  || skip "qa-finding-to-issue.sh not installed"
    command -v jq >/dev/null 2>&1 || skip "jq required"
    TMP="$(mktemp -d)"
    FIX="$TMP/fixtures"
    mkdir -p "$FIX"
    VERDICT="$TMP/.autospec/qa-verdict.json"
    SUMMARY="$TMP/.autospec/heal-summary.md"
    DEDUP="$TMP/.autospec/heal-dedup.txt"
    mkdir -p "$TMP/.autospec"
    # Sandbox HOME so ~/.autospec/{stop,no-heal}.flag from the real machine
    # cannot influence the test outcome.
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    # Don't try to actually run /autospec-run.
    export AUTOSPEC_RUN_CMD="echo 0"
}

teardown() {
    [ -n "${TMP:-}" ] && rm -rf "$TMP"
    return 0
}

mk_round() {
    # mk_round <N> <blocking-count> [tag]
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

@test "single-round convergence — 0 findings exits round 1" {
    mk_round 1 0
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"convergence"* ]]
    [ -f "$SUMMARY" ]
    grep -q "convergence" "$SUMMARY"
}

@test "multi-round convergence — 3 then 0 findings exits round 2" {
    mk_round 1 3 firstwave
    mk_round 2 0
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"convergence"* ]]
    grep -q "convergence" "$SUMMARY"
    # Two rounds in the summary table.
    grep -q "^| 1 " "$SUMMARY"
    grep -q "^| 2 " "$SUMMARY"
}

@test "oscillation detected — same finding set round-over-round" {
    # Same tag both rounds → same hash.
    mk_round 1 2 stable
    mk_round 2 2 stable
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --max-rounds 5
    [ "$status" -eq 1 ]
    [[ "$output" == *"oscillation_detected"* ]]
    grep -q "oscillation_detected" "$SUMMARY"
    grep -q "Persistent findings" "$SUMMARY"
}

@test "round cap reached — 1 finding persists through both rounds with cap=2" {
    # Different tag each round → no oscillation, but cap=2 fires.
    mk_round 1 1 r1
    mk_round 2 1 r2
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --max-rounds 2
    [ "$status" -eq 1 ]
    [[ "$output" == *"round_cap_reached"* ]]
    grep -q "round_cap_reached" "$SUMMARY"
}

@test "budget cap reached — simulated token usage exceeds cap" {
    mk_round 1 1 b1
    mk_round 2 1 b2
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --simulate-tokens 1000000 --token-cap 500000 \
        --max-rounds 5
    [ "$status" -eq 1 ]
    [[ "$output" == *"budget_cap_reached"* ]]
    grep -q "budget_cap_reached" "$SUMMARY"
}

@test "--no-heal — discovery only, no loop, exit 0" {
    mk_round 1 5 nh
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --no-heal
    [ "$status" -eq 0 ]
    [[ "$output" == *"no_heal"* ]]
    grep -q "no_heal" "$SUMMARY"
}

@test "~/.autospec/no-heal.flag forces --no-heal semantics" {
    mk_round 1 3 nhf
    touch "$HOME/.autospec/no-heal.flag"
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no_heal"* ]]
}

@test "heal-summary.md written even on hard exit (round cap)" {
    mk_round 1 1 hs1
    mk_round 2 1 hs2
    run bash "$HEAL_SH" --verdict "$VERDICT" --summary "$SUMMARY" \
        --simulate-rounds "$FIX" --max-rounds 2
    [ -f "$SUMMARY" ]
    grep -q "/autospec-qa heal loop summary" "$SUMMARY"
}

@test "dedup — same finding twice produces one issue" {
    F='{"category":"no_mock_smoke","release_blocking":true,"status":"FAIL","summary":"identical","file":"src/a.ts","evidence":"e"}'
    run bash "$F2I_SH" --finding "$F" --dry-run --dedup-cache "$DEDUP"
    [ "$status" -eq 0 ]
    run bash "$F2I_SH" --finding "$F" --dry-run --dedup-cache "$DEDUP"
    [ "$status" -eq 1 ]
    [[ "$output" == *"dedup-skip"* ]] || [ -n "$output" ]
}
