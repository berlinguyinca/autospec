#!/usr/bin/env bats
# gap-remediation-loop.bats — tests for gap-remediation-loop.sh
# Covers: dedupe vs open issue, dedupe vs docs:drift, round-cap, convergence,
# skip-flag, malformed/empty JSON.
#
# Run: bats skills/autospec-shared/tests/unit/gap-remediation-loop.bats

LOOP="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/gap-remediation-loop.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
    export AUTOSPEC_GAP_REPO="testorg/testrepo"
    # Stub gh: records `issue create` calls, serves a configurable open-issue list.
    mkdir -p "$TEST_TMP/bin"
    export GH_CREATE_LOG="$TEST_TMP/gh-create.log"
    export GH_ISSUE_LIST_JSON="$TEST_TMP/open-issues.json"
    printf '[]\n' > "$GH_ISSUE_LIST_JSON"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  *"issue list"*)
    cat "$GH_ISSUE_LIST_JSON"
    exit 0 ;;
  *"label create"*)
    exit 0 ;;
  *"issue create"*)
    # Emit a fake URL ending in an issue number and log the invocation.
    printf '%s\n' "$*" >> "$GH_CREATE_LOG"
    echo "https://github.com/testorg/testrepo/issues/999"
    exit 0 ;;
  *"issue edit"*)
    exit 0 ;;
  *"repo view"*)
    echo "testorg/testrepo"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"

    # A clean gap that has no dedupe collision.
    cat > "$TEST_TMP/gaps.json" <<'EOF'
[{"gap_id":"G1","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"trailing pipe bug","body":"fix it","dedupe_key":"cross-repo-search-trailing-pipe"}]
EOF
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "gap-remediation-loop.sh is executable" {
    [ -x "$LOOP" ]
}

@test "--help exits 0 and prints Usage" {
    run bash "$LOOP" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

@test "files a survivor and prints survivor count 1" {
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=1"* ]]
    grep -q "issue create" "$GH_CREATE_LOG"
    ! grep -q "auto-implement" "$GH_CREATE_LOG"
    grep -q "needs-classify" "$GH_CREATE_LOG"
    grep -q "gap-remediation" "$GH_CREATE_LOG"
    grep -q "priority:high" "$GH_CREATE_LOG"
    grep -q "origin:self" "$GH_CREATE_LOG"
}

@test "dedupes against an open issue carrying the same dedupe_key in its body" {
    cat > "$GH_ISSUE_LIST_JSON" <<'EOF'
[{"number":42,"title":"old","body":"dedupe_key: cross-repo-search-trailing-pipe","labels":[{"name":"auto-implement"}]}]
EOF
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "dedupes against an open issue carrying an active docs:drift label with matching title" {
    cat > "$GH_ISSUE_LIST_JSON" <<'EOF'
[{"number":43,"title":"trailing pipe bug","body":"unrelated","labels":[{"name":"docs:drift"}]}]
EOF
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "convergence: empty gap array files nothing and reports survivors=0" {
    printf '[]\n' > "$TEST_TMP/gaps.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
}

@test "malformed JSON is treated as 0 survivors and warns" {
    printf 'not json at all\n' > "$TEST_TMP/gaps.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"WARN"* ]]
}

@test "round-cap: refuses to file when round-state already hit AUTOSPEC_GAP_MAX_ROUNDS" {
    export AUTOSPEC_GAP_MAX_ROUNDS=2
    printf '{"round":2}\n' > "$TEST_TMP/gap-round-state.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"round cap"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "round-state increments after a filing round" {
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/gap-round-state.json"
    [[ "$output" == *'"round": 1'* ]] || [[ "$output" == *'"round":1'* ]]
}

@test "intra-run dedupe: two identical gaps in one JSON file only one survivor" {
    cat > "$TEST_TMP/gaps.json" <<'EOF'
[{"gap_id":"G1","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"trailing pipe bug","body":"fix it","dedupe_key":"cross-repo-search-trailing-pipe"},
 {"gap_id":"G2","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"trailing pipe bug","body":"fix it again","dedupe_key":"cross-repo-search-trailing-pipe"}]
EOF
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=1"* ]]
    [ "$(grep -c "issue create" "$GH_CREATE_LOG")" -eq 1 ]
}

@test "skip flag ~/.autospec/no-review.flag short-circuits to survivors=0" {
    touch "$TEST_TMP/no-review.flag"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"skip"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}
