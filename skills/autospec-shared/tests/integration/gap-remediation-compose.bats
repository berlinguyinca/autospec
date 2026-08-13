#!/usr/bin/env bats
# gap-remediation-compose.bats — end-to-end integration smoke for the
# end-of-run gap-remediation feature (gap-remediation Task 7).
#
# Proves the wired pieces COMPOSE with the real scripts (no mocks of autospec
# code; only `gh`, the external service, is stubbed — matching the per-script
# unit-test boundary):
#
#   raw review findings
#     -> emit-gaps.sh            (filter false-positives, shape gap JSON)
#     -> gap-json-lib.gap_validate_object   (every shaped gap is schema-valid)
#     -> gap-remediation-loop.sh (dedupe vs open issues, file survivors, converge)
#
# This is deliberately a cross-script flow: the unit suites exercise each script
# in isolation, but only this test runs the exact emitted artifact of one script
# straight into the next, the way Phase 5.5 / `/autospec-review --remediation` do.
#
# Run: bats skills/autospec-shared/tests/integration/gap-remediation-compose.bats

SHARED="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
EMIT="$SHARED/scripts/emit-gaps.sh"
LIB="$SHARED/scripts/gap-json-lib.sh"
LOOP="$SHARED/scripts/gap-remediation-loop.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
    export AUTOSPEC_GAP_REPO="testorg/testrepo"

    # Stub gh exactly as the gap-remediation-loop unit suite does: serve a
    # configurable open-issue list and log `issue create` calls.
    mkdir -p "$TEST_TMP/bin"
    export GH_CREATE_LOG="$TEST_TMP/gh-create.log"
    export GH_ISSUE_LIST_JSON="$TEST_TMP/open-issues.json"
    printf '[]\n' > "$GH_ISSUE_LIST_JSON"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  *"issue list"*)   cat "$GH_ISSUE_LIST_JSON"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue create"*)
    printf '%s\n' "$*" >> "$GH_CREATE_LOG"
    echo "https://github.com/testorg/testrepo/issues/999"; exit 0 ;;
  *"issue edit"*)   exit 0 ;;
  *"repo view"*)    echo "testorg/testrepo"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"

    # Raw review findings as `/autospec-review --remediation` would produce:
    # one genuine correctness defect (keep) + one flagged false-positive (drop).
    cat > "$TEST_TMP/findings.json" <<'EOF'
[
  {"dimension":"correctness","severity":"high","file":"cross-repo-search.sh","line":77,
   "title":"trailing pipe matches every line on BSD grep","body":"build pattern drops trailing \\|","verdict":"keep","dedupe_key":"cross-repo-search-trailing-pipe"},
  {"dimension":"test-quality","severity":"low","file":"x.sh","line":1,
   "title":"phantom defect","body":"reviewer hallucinated","verdict":"false_positive","dedupe_key":"phantom"}
]
EOF
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "compose: emit-gaps output is schema-valid and drives the loop to file exactly the surviving gap" {
    # Stage 1: shape raw findings into the gap contract.
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/gaps.json" ]

    # The false-positive is dropped; exactly the genuine defect survives.
    [ "$(jq 'length' "$TEST_TMP/gaps.json")" -eq 1 ]
    [ "$(jq -r '.[0].dedupe_key' "$TEST_TMP/gaps.json")" = "cross-repo-search-trailing-pipe" ]

    # Stage 2: every emitted gap validates against the shared schema lib.
    obj="$(jq -c '.[0]' "$TEST_TMP/gaps.json")"
    run bash -c ". '$LIB'; gap_validate_object '$obj'"
    [ "$status" -eq 0 ]

    # Stage 3: feed the SAME emitted artifact straight into the driver.
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=1"* ]]
    [[ "$output" == *"filed=1"* ]]

    # A pending-classification gap-remediation issue was filed for the survivor.
    grep -q "issue create" "$GH_CREATE_LOG"
    ! grep -q "auto-implement" "$GH_CREATE_LOG"
    grep -q "needs-classify" "$GH_CREATE_LOG"
    grep -q "gap-remediation" "$GH_CREATE_LOG"

    # Round state advanced exactly once.
    [[ "$(cat "$TEST_TMP/gap-round-state.json")" == *'"round": 1'* ]] || \
        [[ "$(cat "$TEST_TMP/gap-round-state.json")" == *'"round":1'* ]]
}

@test "compose: a gap whose dedupe_key already exists on an open issue is suppressed end-to-end (convergence)" {
    # An open auto-implement issue already carries the survivor's dedupe_key.
    cat > "$GH_ISSUE_LIST_JSON" <<'EOF'
[{"number":42,"title":"old gap","body":"dedupe_key: cross-repo-search-trailing-pipe","labels":[{"name":"auto-implement"}]}]
EOF

    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    [ "$(jq 'length' "$TEST_TMP/gaps.json")" -eq 1 ]

    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    # Nothing filed — the chain converges instead of re-filing a known gap.
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "compose: an all-false-positive review converges with no issue filed" {
    cat > "$TEST_TMP/findings.json" <<'EOF'
[{"dimension":"correctness","severity":"low","file":"y.sh","line":1,
  "title":"nothing real","body":"noise","verdict":"false_positive","dedupe_key":"noise"}]
EOF
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    [ "$(jq 'length' "$TEST_TMP/gaps.json")" -eq 0 ]

    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}
