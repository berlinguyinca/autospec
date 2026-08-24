#!/usr/bin/env bats
# The dedicated Git ref is the claim CAS linearization point. GitHub comments
# are audit projections and must never decide ownership.

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

claim_state() {
    "$AUTOSPEC" claim state "$@"
}

claim_acquire() {
    "$AUTOSPEC" claim acquire "$@"
}

claim_oid() {
    git --git-dir "$CLAIM_REMOTE" rev-parse refs/autospec/claims/issue-42
}

acquire_worker_a() {
    claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-a --branch feat/worker-a
}

@test "read returns the authoritative Git-ref owner" {
    run acquire_worker_a
    [ "$status" -eq 0 ]

    run claim_state read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [[ "$output" == *'"worker_id":"worker-a"'* ]]
    [[ "$output" == *'"branch":"feat/worker-a"'* ]]
    [[ "$output" == *'"claim_id":"claim-'* ]]
}

@test "one exact successor advances the same issue ref" {
    acquired="$(acquire_worker_a)"
    claim_id="$(printf '%s' "$acquired" | jq -r .claim_id)"
    before="$(claim_oid)"

    run claim_state upsert --issue 42 --repo testorg/testrepo \
        --worker-id worker-a --claim-id "$claim_id" --branch feat/worker-a \
        --state claimed --step worktree_ready
    [ "$status" -eq 0 ]
    [ "$(claim_oid)" != "$before" ]
    [ "$(git --git-dir "$CLAIM_REMOTE" for-each-ref --format='%(refname)' \
        refs/autospec/claims/issue-42 | wc -l | tr -d ' ')" -eq 1 ]
}

@test "a fresh competing worker loses without advancing the winner ref" {
    acquire_worker_a >/dev/null
    printf 'auto-implement\nctx:32k\nsafety:reviewed\n' > "$LABELS"
    before="$(claim_oid)"

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-b --branch feat/worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed":false'* ]]
    [[ "$output" == *'"reason":"claim_lost"'* ]]
    [ "$(claim_oid)" = "$before" ]
}

@test "a dotted worker id cannot collide with a different owner" {
    acquire_worker_a >/dev/null
    printf 'auto-implement\nctx:32k\nsafety:reviewed\n' > "$LABELS"

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id 'mac.lan:bob:monitor:1' --branch feat/dotted
    [ "$status" -eq 2 ]
    [[ "$output" == *'"reason":"claim_lost"'* ]]

    run claim_state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"worker_id":"worker-a"'* ]]
    [[ "$output" != *'mac.lan:bob:monitor:1'* ]]
}

@test "the first successful acquire remains the only fresh generation owner" {
    run acquire_worker_a
    [ "$status" -eq 0 ]
    first_claim="$(printf '%s' "$output" | jq -r .claim_id)"

    run claim_acquire --issue 42 --repo testorg/testrepo \
        --worker-id worker-b --branch feat/worker-b
    [ "$status" -eq 2 ]

    run claim_state read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .claim_id)" = "$first_claim" ]
}
