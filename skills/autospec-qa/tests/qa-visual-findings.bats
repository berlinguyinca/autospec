#!/usr/bin/env bats
# qa-visual-findings.bats — vision verdicts -> qa-verdict visual_fidelity findings.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../.." && pwd)"
    Q="$REPO_ROOT/skills/autospec-qa/scripts/qa-visual-findings.sh"
    TMP="$(mktemp -d /tmp/autospec-visual-XXXXXX)"
    BASH_BIN="$(command -v bash)"
}
teardown() { rm -rf "$TMP"; }

run_q() { printf '%s' "$1" | bash "$Q" ${2:+--blocking-on "$2"}; }

@test "FAIL verdict -> release-blocking visual_fidelity finding" {
    out="$(run_q '[{"route":"/checkout","viewport":"desktop","status":"FAIL","issues":["spacing != token"]}]')"
    [ "$(printf '%s' "$out" | jq -r '.[0].category')" = "visual_fidelity" ]
    [ "$(printf '%s' "$out" | jq -r '.[0].release_blocking')" = "true" ]
    printf '%s' "$out" | jq -e '.[0].summary | test("checkout")' >/dev/null
    [ "$(printf '%s' "$out" | jq -r '.[0].evidence')" = "docs/assets/screenshots/checkout__desktop.png" ]
}

@test "PARTIAL is advisory by default (release_blocking=false)" {
    out="$(run_q '[{"route":"/","viewport":"mobile","status":"PARTIAL","issues":["font off"]}]')"
    [ "$(printf '%s' "$out" | jq -r '.[0].release_blocking')" = "false" ]
    [ "$(printf '%s' "$out" | jq -r '.[0].evidence')" = "docs/assets/screenshots/root__mobile.png" ]
}

@test "PARTIAL becomes blocking with --blocking-on PARTIAL" {
    out="$(run_q '[{"route":"/","viewport":"mobile","status":"PARTIAL","issues":["x"]}]' PARTIAL)"
    [ "$(printf '%s' "$out" | jq -r '.[0].release_blocking')" = "true" ]
}

@test "PASS verdicts produce no findings" {
    out="$(run_q '[{"route":"/about","viewport":"desktop","status":"PASS","issues":[]}]')"
    [ "$(printf '%s' "$out" | jq -c '.')" = "[]" ]
}

@test "mixed verdicts keep only non-PASS" {
    out="$(run_q '[{"route":"/a","status":"FAIL","issues":["i"]},{"route":"/b","status":"PASS","issues":[]},{"route":"/c","status":"PARTIAL","issues":["j"]}]')"
    [ "$(printf '%s' "$out" | jq 'length')" = "2" ]
}

@test "empty input -> []" {
    [ "$(printf '' | bash "$Q")" = "[]" ]
}

@test "issue-less verdict still gets a summary" {
    out="$(run_q '[{"route":"/x","viewport":"desktop","status":"FAIL","issues":[]}]')"
    printf '%s' "$out" | jq -e '.[0].summary | test("DESIGN.md")' >/dev/null
}

@test "responsive defect category is preserved in finding" {
    out="$(run_q '[{"route":"/dashboard","viewport":"mobile","status":"FAIL","category":"clipped-tab","issues":["tab is clipped"]}]')"
    [ "$(printf '%s' "$out" | jq -r '.[0].category')" = "clipped-tab" ]
    [ "$(printf '%s' "$out" | jq -r '.[0].release_blocking')" = "true" ]
}

@test "tablet responsive defect blocks even when category is supplied" {
    out="$(run_q '[{"route":"/reports","viewport":"tablet","status":"FAIL","category":"unresponsive-table","issues":["table overflows"]}]')"
    [ "$(printf '%s' "$out" | jq -r '.[0].release_blocking')" = "true" ]
}

@test "jq missing -> exit 2 (fail-closed)" {
    mkdir -p "$TMP/empty"
    run bash -c "printf '%s' '[]' | env PATH=\"$TMP/empty\" \"$BASH_BIN\" \"$Q\""
    [ "$status" -eq 2 ]
}
