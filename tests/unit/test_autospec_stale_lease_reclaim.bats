#!/usr/bin/env bats
# Cross-machine reclaim is decided by the authoritative Git-ref record's
# updated_at and ttl_seconds. Audit comment ordering is deliberately irrelevant.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    TEST_TMP="$(mktemp -d)"
    LABELS="$TEST_TMP/labels.txt"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    CLAIM_REMOTE="$TEST_TMP/claim-remote.git"
    printf 'auto-implement\nctx:32k\nsafety:reviewed\n' > "$LABELS"
    printf '[]\n' > "$COMMENTS"
    git init --bare -q "$CLAIM_REMOTE"

    mkdir -p "$TEST_TMP/bin"
    cp "$REPO_ROOT/tests/fixtures/gh-mock/gh" "$TEST_TMP/bin/gh"
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:/usr/bin:/bin"
    export AUTOSPEC_TEST_LABELS="$LABELS"
    export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
    export AUTOSPEC_TEST_CALLS="$CALLS"
    export AUTOSPEC_GH_API_RETRY_SLEEP=0
    export AUTOSPEC_CLAIM_GIT_REMOTE="$CLAIM_REMOTE"
    export AUTOSPEC_CLAIM_GIT_STATE_DIR="$TEST_TMP/claim-state"
    export AUTOSPEC_HEARTBEAT_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_CLAIM_CONFIRM_READS=1
    export AUTOSPEC_CLAIM_SETTLE_MILLIS=0
}

teardown() {
    rm -rf "$TEST_TMP"
}

claim_acquire() {
    "$AUTOSPEC" claim acquire "$@"
}

iso_ago() {
    date -u -j -v-"$1"S +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "@$(( $(date -u +%s) - $1 ))" +'%Y-%m-%dT%H:%M:%SZ'
}

seed_claim_ref() {
    worker_id="$1"
    branch="$2"
    claim_id="$3"
    updated_at="$4"
    message="$TEST_TMP/claim-message"
    tree="$(git --git-dir "$CLAIM_REMOTE" mktree </dev/null)"
    cat > "$message" <<EOF
autospec-claim-ledger-v1
generation=fixture-$claim_id

<!-- autospec-run-state:begin -->
{"schema":1,"repo":"testorg/testrepo","issue":42,"worker_id":"$worker_id","state":"claimed","branch":"$branch","pr":"","step":"claimed","paths":[],"claimed_at":"$updated_at","updated_at":"$updated_at","ttl_seconds":10800,"claim_id":"$claim_id"}
<!-- autospec-run-state:end -->
EOF
    oid="$(
        GIT_AUTHOR_NAME='Autospec Test' \
        GIT_AUTHOR_EMAIL='autospec-test@localhost' \
        GIT_COMMITTER_NAME='Autospec Test' \
        GIT_COMMITTER_EMAIL='autospec-test@localhost' \
        git --git-dir "$CLAIM_REMOTE" commit-tree "$tree" -F "$message"
    )"
    git --git-dir "$CLAIM_REMOTE" update-ref refs/autospec/claims/issue-42 "$oid"
}

@test "a stale authoritative generation is reclaimed by a new worker" {
    seed_claim_ref worker-a feat/worker-a claim-a "$(iso_ago 20000)"

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-b --branch feat/worker-b
    [ "$status" -eq 0 ]
    [[ "$output" == *'"claimed":true'* ]]
    [[ "$output" == *'"worker_id":"worker-b"'* ]]

    run "$AUTOSPEC" claim state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"worker_id":"worker-b"'* ]]
    [[ "$output" != *'"claim_id":"claim-a"'* ]]
}

@test "a fresh authoritative generation rejects another worker" {
    seed_claim_ref worker-a feat/worker-a claim-a "$(iso_ago 60)"

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-b --branch feat/worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed":false'* ]]
    [[ "$output" == *'"reason":"claim_lost"'* ]]

    run "$AUTOSPEC" claim state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"claim_id":"claim-a"'* ]]
}

@test "audit comment ordering cannot steal a fresh Git-ref generation" {
    fresh="$(iso_ago 30)"
    seed_claim_ref worker-a feat/worker-a claim-a "$fresh"
    cat > "$COMMENTS" <<JSON
[
  {"id":101,"updated_at":"$fresh","body":"worker-b audit projection"},
  {"id":100,"updated_at":"$fresh","body":"worker-a audit projection"}
]
JSON

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-b --branch feat/worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"reason":"claim_lost"'* ]]

    run "$AUTOSPEC" claim state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"worker_id":"worker-a"'* ]]
    [ "$(jq -r '.[0].id' "$COMMENTS")" = 101 ]
}
