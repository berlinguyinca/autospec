#!/usr/bin/env bats
# tests/unit/test_autospec_stale_lease_reclaim.bats — cross-machine stale-lease
# reclaim keyed on the lock comment's SERVER-SIDE updated_at vs the reclaim TTL.
#
# Fixed evaluation order (spec §Critical-improvement fold-in):
#   (1) determine the lowest-id marked lock comment = current owner;
#   (2) if I am NOT that owner and the lowest-id lock is FRESH -> claim lost,
#       self-clean my own comment, exit 2 (a fresh winner is never a stale lease);
#   (3) ONLY if the lowest-id lock is STALE -> reclaim: upsert my own worker_id +
#       fresh updated_at, re-run read-back verify (re-resolve lowest id), exit 0.
#
# Staleness is judged strictly on the SERVER updated_at returned by
# `gh api .../comments`, never a local clock stored in the lock body.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    CLAIM="$REPO_ROOT/skills/autospec-run/scripts/claim-issue.sh"
    TEST_TMP="$(mktemp -d)"
    LABELS="$TEST_TMP/labels.txt"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    printf 'auto-implement\nctx:32k\nsafety:reviewed\n' > "$LABELS"
    printf '[]\n' > "$COMMENTS"

    # Shared PATH-shadow gh stub (see tests/fixtures/gh-mock/gh). It returns the
    # raw comments JSON for `api .../comments`, so updated_at fields seeded into
    # the fixture flow through to the script's server-timestamp read.
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

# iso UTC timestamp N seconds in the past (portable BSD/GNU).
iso_ago() {
    date -u -j -v-"$1"S +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "@$(( $(date -u +%s) - $1 ))" +'%Y-%m-%dT%H:%M:%SZ'
}

# Seed a single marked lock comment (id 100) owned by worker-a with the given
# server updated_at. Mirrors the descending-array shape used elsewhere, but a
# single comment suffices for the owner/staleness decision.
seed_owned_lock() {
    updated_at="$1"
    cat > "$COMMENTS" <<JSON
[
  {
    "id": 100,
    "updated_at": "$updated_at",
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\",\"updated_at\":\"$updated_at\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"
  }
]
JSON
}

@test "(a) stale lowest-id lock: a new worker reclaims, exit 0, refreshes updated_at" {
    # worker-a's lock server updated_at aged well past the default 10800s TTL.
    seed_owned_lock "$(iso_ago 20000)"

    run bash -c 'bash "$0" --issue 42 --repo testorg/testrepo --worker-id worker-b --branch feat/x 2>&1' "$CLAIM"
    [ "$status" -eq 0 ]
    [[ "$output" == *'"claimed": true'* ]]
    [[ "$output" == *'"worker_id": "worker-b"'* ]]
    # reclaim is logged distinguishably from claim lost
    [[ "$output" == *'stale lease reclaimed'* ]]
    [[ "$output" != *'claim lost'* ]]
    # the surviving marked lock is now owned by worker-b with a fresh updated_at
    run jq -r 'map(select((.body//"")|contains("autospec-run-state:begin")))
               | sort_by(.id) | .[0].body' "$COMMENTS"
    [[ "$output" == *'worker-b'* ]]
    [[ "$output" != *'worker-a'* ]]
}

@test "(b) fresh lowest-id lock owned by another worker: exit 2, no reclaim" {
    # worker-a's lock is well within TTL: a live-but-slow worker must NOT be
    # reclaimed. worker-b loses and self-cleans.
    seed_owned_lock "$(iso_ago 60)"

    run bash "$CLAIM" --issue 42 --repo testorg/testrepo --worker-id worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed": false'* ]]
    [[ "$output" == *'"reason": "claim_lost"'* ]]
    # never logged a reclaim for a fresh lease
    ! grep -q 'stale lease reclaimed' "$CALLS"
}

@test "(c) ordering guard: higher-id worker vs FRESH lower-id lock -> lost, no reclaim regardless of updated_at" {
    # Two marked comments: lowest id 100 (worker-a, FRESH) and higher id 101
    # (worker-b). worker-b is the higher id; it must conclude claim lost and must
    # NOT mistake worker-a's fresh lower-id comment for a stale lease.
    fresh="$(iso_ago 30)"
    cat > "$COMMENTS" <<JSON
[
  {
    "id": 101,
    "updated_at": "$fresh",
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"claimed_at\":\"$fresh\",\"updated_at\":\"$fresh\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 100,
    "updated_at": "$fresh",
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"claimed_at\":\"$fresh\",\"updated_at\":\"$fresh\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"
  }
]
JSON
    run bash "$CLAIM" --issue 42 --repo testorg/testrepo --worker-id worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"reason": "claim_lost"'* ]]
    ! grep -q 'stale lease reclaimed' "$CALLS"
    # the fresh lower-id winner (100) is never deleted by the loser
    ! grep -q 'api repos/testorg/testrepo/issues/comments/100 -X DELETE' "$CALLS"
    # and its body is never overwritten: no lease theft of a fresh lower-id lock
    run jq -r 'map(select(.id == 100)) | .[0].body' "$COMMENTS"
    [[ "$output" == *'worker-a'* ]]
    [[ "$output" != *'worker-b'* ]]
}
