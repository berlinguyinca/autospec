#!/usr/bin/env bats
# tests/compute-release-verdict.bats — deterministic verdict computation
# from qa-verdict.json. Covers the exact failure modes that produced the
# false-PASS regression in PR #633.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/compute-release-verdict.sh"

setup() {
    TMP=$(mktemp -d)
    HEAD_SHA="0123456789abcdef0123456789abcdef01234567"
    VERDICT="$TMP/qa-verdict.json"
}

teardown() {
    rm -rf "$TMP"
}

write_verdict() {
    # $1 = JSON body
    printf '%s' "$1" > "$VERDICT"
}

run_script() {
    run bash "$SCRIPT" "$VERDICT" "$HEAD_SHA"
}

@test "FAIL when verdict file is missing" {
    run bash "$SCRIPT" "$TMP/does-not-exist.json" "$HEAD_SHA"
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
    [[ "$output" == *"not produced by Stage 7"* ]]
}

@test "FAIL when verdict file is not valid JSON" {
    write_verdict 'not json at all {'
    run_script
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
    [[ "$output" == *"not valid JSON"* ]]
}

@test "FAIL when head_sha is missing" {
    write_verdict '{"live_app_proof": true, "findings": []}'
    run_script
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
    [[ "$output" == *"missing head_sha"* ]]
}

@test "FAIL when head_sha is stale relative to HEAD" {
    write_verdict "{\"head_sha\": \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\", \"live_app_proof\": true, \"findings\": []}"
    run_script
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
    [[ "$output" == *"stale relative to HEAD"* ]]
}

@test "PARTIAL when live_app_proof is false" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": false, \"findings\": []}"
    run_script
    [ "$status" -eq 1 ]
    [[ "$output" == *"PARTIAL"* ]]
    [[ "$output" == *"live_app_proof is false"* ]]
}

@test "PARTIAL when one release_blocking finding is NOT_TESTED" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": true, \"findings\": [
        {\"category\": \"no_mock_smoke\", \"release_blocking\": true, \"status\": \"NOT_TESTED\", \"summary\": \"e2e:prod smoke missing\"}
    ]}"
    run_script
    [ "$status" -eq 1 ]
    [[ "$output" == *"PARTIAL"* ]]
    [[ "$output" == *"e2e:prod smoke missing"* ]]
}

@test "PARTIAL when one release_blocking finding has status PARTIAL" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": true, \"findings\": [
        {\"category\": \"mutation_proof_missing\", \"release_blocking\": true, \"status\": \"PARTIAL\", \"summary\": \"checkout flow has no mutation proof\"}
    ]}"
    run_script
    [ "$status" -eq 1 ]
    [[ "$output" == *"PARTIAL"* ]]
    [[ "$output" == *"checkout flow has no mutation proof"* ]]
}

@test "FAIL when one release_blocking finding has status FAIL" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": true, \"findings\": [
        {\"category\": \"outsourced_implementation\", \"release_blocking\": true, \"status\": \"FAIL\", \"summary\": \"classifier delegates to ClassyFire API\"}
    ]}"
    run_script
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
    [[ "$output" == *"ClassyFire"* ]]
}

@test "FAIL takes precedence over live_app_proof=false" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": false, \"findings\": [
        {\"category\": \"outsourced_implementation\", \"release_blocking\": true, \"status\": \"FAIL\", \"summary\": \"product-identity violation\"}
    ]}"
    run_script
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
}

@test "PASS when verdict file is fresh, live_app_proof=true, no blocking findings" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": true, \"findings\": []}"
    run_script
    [ "$status" -eq 0 ]
    [[ "$output" == *"PASS"* ]]
}

@test "PASS when only non-release-blocking findings exist" {
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": true, \"findings\": [
        {\"category\": \"accessibility\", \"release_blocking\": false, \"status\": \"PARTIAL\", \"summary\": \"contrast warning\"}
    ]}"
    run_script
    [ "$status" -eq 0 ]
    [[ "$output" == *"PASS"* ]]
}

@test "regression: false-PASS scenario from PR #633 is rejected" {
    # Exact failure pattern: agent emitted PASS with NOT_TESTED browser smoke.
    write_verdict "{\"head_sha\": \"${HEAD_SHA}\", \"live_app_proof\": false, \"findings\": [
        {\"category\": \"no_mock_smoke\", \"release_blocking\": true, \"status\": \"NOT_TESTED\", \"summary\": \"Live e2e:prod smoke against tidyboard.org\"},
        {\"category\": \"automated_test_gap\", \"release_blocking\": true, \"status\": \"NOT_TESTED\", \"summary\": \"Browser-level interactive verification\"}
    ]}"
    run_script
    [ "$status" -ne 0 ]
    [[ "$output" != *"PASS"* ]]
}
