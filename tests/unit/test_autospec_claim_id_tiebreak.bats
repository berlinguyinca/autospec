#!/usr/bin/env bats
# tests/unit/test_autospec_claim_id_tiebreak.bats — earliest-comment-id CAS tiebreak.
#
# Proves the lock comment is selected by lowest numeric id (the CAS
# linearization point), never by API/array order:
#   (a) run-state.sh `read` returns the body of the lowest id (100), not array order;
#   (b) the worker owning the higher id (101) loses, exits 2 `claim_lost`,
#       and DELETEs only its own (101) comment — never the winner's (100);
#   (c) dedup never DELETEs the lowest id (100).
# Covers both the both-create and both-patch races.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/run-state.sh"
    CLAIM="$REPO_ROOT/skills/autospec-run/scripts/claim-issue.sh"
    TEST_TMP="$(mktemp -d)"
    LABELS="$TEST_TMP/labels.txt"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    printf 'auto-implement\nctx:32k\n' > "$LABELS"
    printf '[]\n' > "$COMMENTS"

    # Shared PATH-shadow gh stub (see tests/fixtures/gh-mock/gh).
    mkdir -p "$TEST_TMP/bin"
    cp "$REPO_ROOT/tests/fixtures/gh-mock/gh" "$TEST_TMP/bin/gh"
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_TEST_LABELS="$LABELS"
    export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
    export AUTOSPEC_TEST_CALLS="$CALLS"
    export AUTOSPEC_TEST_FORCE_OWNER=""
    export AUTOSPEC_GH_API_RETRY_SLEEP=0
}

teardown() {
    rm -rf "$TEST_TMP"
}

# Seed two marked lock comments in DESCENDING array order so a naive
# .[0]/sed-1p selection would pick the higher id. Only sort_by(.id) gets 100.
seed_descending_array_order() {
    cat > "$COMMENTS" <<'JSON'
[
  {
    "id": 101,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:01Z\",\"updated_at\":\"2026-01-01T00:00:01Z\"}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 100,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\",\"updated_at\":\"2026-01-01T00:00:00Z\"}\n<!-- autospec-run-state:end -->"
  }
]
JSON
}

@test "read returns the body of the lowest id, not array order" {
    seed_descending_array_order
    run bash "$SCRIPT" read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [[ "$output" == *'"worker_id": "worker-a"'* ]]
    [[ "$output" != *'"worker_id": "worker-b"'* ]]
}

@test "dedup keeps the lowest id (100) and deletes the higher (101)" {
    seed_descending_array_order
    run bash "$SCRIPT" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state worktree_ready
    [ "$status" -eq 0 ]

    # exactly one marked comment survives
    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
    # the survivor is the lowest id (100), never 101
    run jq -r '.[0].id' "$COMMENTS"
    [ "$output" = "100" ]
    # 101 was DELETEd via REST, 100 was not
    grep -q 'api repos/testorg/testrepo/issues/comments/101 -X DELETE' "$CALLS"
    ! grep -q 'api repos/testorg/testrepo/issues/comments/100 -X DELETE' "$CALLS"
}

@test "both-patch race: higher-id worker loses, deletes only its own comment, exits 2" {
    # worker-a holds the lowest-id lock (100). worker-b genuinely owns the
    # higher id (101). worker-b runs the claim path; FORCE_OWNER keeps worker-a
    # as the owner of the patched lowest id, so worker-b's read-back shows it
    # lost. worker-b must DELETE only its OWN comment (101), never the winner's
    # lowest id (100).
    seed_descending_array_order
    printf 'auto-implement\nctx:32k\n' > "$LABELS"

    AUTOSPEC_TEST_FORCE_OWNER=worker-a run bash "$CLAIM" --issue 42 --repo testorg/testrepo --worker-id worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed": false'* ]]
    [[ "$output" == *'"reason": "claim_lost"'* ]]
    # loser self-deletes its OWN comment (101) ...
    grep -q 'api repos/testorg/testrepo/issues/comments/101 -X DELETE' "$CALLS"
    # ... and never the winner's lowest id (100)
    ! grep -q 'api repos/testorg/testrepo/issues/comments/100 -X DELETE' "$CALLS"
}

@test "dotted worker_id self-cleans only its own comment, never a near-collision" {
    # Cross-machine correctness: a worker_id with a regex metacharacter must
    # match by LITERAL equality, not jq test(). winner-a holds the lowest-id
    # lock (100) and wins via FORCE_OWNER. The loser's worker_id contains a dot
    # (mac.lan:bob:monitor:1, its own comment is 101). A DIFFERENT worker owns a
    # near-collision id (macXlan:bob:monitor:1, comment 102) where `.` would
    # regex-match `X`. The loser must DELETE only its OWN comment (101) and never
    # the near-collision's (102) nor the winner's (100).
    cat > "$COMMENTS" <<'JSON'
[
  {
    "id": 100,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"winner-a\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\",\"updated_at\":\"2026-01-01T00:00:00Z\"}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 101,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"mac.lan:bob:monitor:1\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:01Z\",\"updated_at\":\"2026-01-01T00:00:01Z\"}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 102,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"macXlan:bob:monitor:1\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:02Z\",\"updated_at\":\"2026-01-01T00:00:02Z\"}\n<!-- autospec-run-state:end -->"
  }
]
JSON
    printf 'auto-implement\nctx:32k\n' > "$LABELS"

    AUTOSPEC_TEST_FORCE_OWNER=winner-a run bash "$CLAIM" --issue 42 --repo testorg/testrepo --worker-id 'mac.lan:bob:monitor:1'
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed": false'* ]]
    [[ "$output" == *'"reason": "claim_lost"'* ]]
    # loser self-deletes its OWN comment (101) ...
    grep -q 'api repos/testorg/testrepo/issues/comments/101 -X DELETE' "$CALLS"
    # ... and NEVER the near-collision's (102), even though `.` regex-matches `X`
    ! grep -q 'api repos/testorg/testrepo/issues/comments/102 -X DELETE' "$CALLS"
    # ... and never the winner's lowest id (100)
    ! grep -q 'api repos/testorg/testrepo/issues/comments/100 -X DELETE' "$CALLS"
}

@test "both-create race: two distinct marked comments, lower id wins the read" {
    # No pre-existing lock; both workers CREATE a fresh marked comment, ending
    # with two distinct marked comments (100 then 101). read returns the lower id.
    cat > "$COMMENTS" <<'JSON'
[
  {
    "id": 100,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\",\"updated_at\":\"2026-01-01T00:00:00Z\"}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 101,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:01Z\",\"updated_at\":\"2026-01-01T00:00:01Z\"}\n<!-- autospec-run-state:end -->"
  }
]
JSON
    run bash "$SCRIPT" read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [[ "$output" == *'"worker_id": "worker-a"'* ]]
}
