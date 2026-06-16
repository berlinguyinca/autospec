#!/usr/bin/env bats
# tests/test_claim_guard_stale_reclaim.bats — TTL stale-reclaim boundary.
#
# The spec (docs/specs/2026-06-15-claim-guard-concurrent-edit-design.md,
# §"Error handling + reclaim" / §"Testing") names this file explicitly:
#   "a claim with updated_at past TTL is reclaimable; one within TTL is NOT."
# It was previously only covered inline in test_claim_guard_atomicity.bats;
# this is the dedicated, spec-named suite the issue #1069 Phase 5.5 audit
# requires (and that validate.sh's verification commands reference).
#
# Real filesystem store, no mocks (AGENTS.md). AUTOSPEC_STATE_DIR redirects the
# store to a per-test tmp dir so we never touch ~/.autospec. AUTOSPEC_REPO is
# pinned so the slug is stable and assertable without re-deriving it from the
# SUT (self-consistent-fixture trap).

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_CLAIM_GUARD="strict"
    export AUTOSPEC_HOST="testhost"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca_autospec"
}

teardown() {
    rm -rf "$STATE_DIR"
}

# ---------------------------------------------------------------------------
@test "a claim with updated_at past TTL is reclaimable by another session" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-old" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -f "$claim_file" ]

    # Age the claim past TTL by rewriting updated_at far into the past. Pinning a
    # literal timestamp (not now-minus-delta) keeps the fixture independent of
    # the SUT's own clock math.
    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"

    CLAUDE_CODE_SESSION_ID="sess-fresh" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-fresh" ]
    # Reclaim resets acquired_at to the new owner's acquisition time (a fresh
    # lease, not a continuation of the dead owner's).
    acquired="$(jq -r '.acquired_at' "$claim_file")"
    [ "$acquired" != "" ]
    [ "$acquired" != "2000-01-01T00:00:00Z" ]
}

# ---------------------------------------------------------------------------
@test "a claim WITHIN TTL is never reclaimed (a slow-but-live editor is safe)" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-live" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"

    # Fresh claim (updated_at = now) is well within TTL -> intruder blocked.
    CLAUDE_CODE_SESSION_ID="sess-thief" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
    printf '%s\n' "$output" | grep -q 'code_health:claim_conflict'

    # Ownership unchanged: the live editor was NOT stolen.
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-live" ]
}

# ---------------------------------------------------------------------------
@test "a claim refreshed just below the TTL boundary is still NOT reclaimable" {
    # TTL of 1800s; backdate updated_at to 1799s ago (inside TTL by 1s). The
    # owner is effectively a live-but-slow editor; the intruder must back off.
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-live" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"

    near="$(date -u -v-1700S +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '-1700 seconds' +'%Y-%m-%dT%H:%M:%SZ')"
    tmp="$(mktemp)"
    jq --arg u "$near" '.updated_at = $u' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"

    CLAUDE_CODE_SESSION_ID="sess-thief" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-live" ]
}

# ---------------------------------------------------------------------------
@test "a stale claim with a present .lock dir is atomically reclaimed (re-mkdir)" {
    # Reproduce the reclaim path where the dead owner left BOTH the JSON and the
    # .lock dir behind. Reclaim must rmdir+re-mkdir the lock atomically and hand
    # the lease to the fresh session.
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-old" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -d "${CLAIM_ROOT}/skill-autospec-run.lock" ]

    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"

    CLAUDE_CODE_SESSION_ID="sess-fresh" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-fresh" ]
    # The lock dir still exists (held by the new owner), not leaked away.
    [ -d "${CLAIM_ROOT}/skill-autospec-run.lock" ]
}

# ---------------------------------------------------------------------------
@test "status flags a stale claim as 'stale' and a fresh one as 'live'" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-fresh" bash "$GUARD" acquire skills/autospec-explore
    CLAUDE_CODE_SESSION_ID="sess-old" bash "$GUARD" acquire skills/autospec-run
    stale_file="${CLAIM_ROOT}/skill-autospec-run.json"
    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$stale_file" > "$tmp" && mv "$tmp" "$stale_file"

    run bash "$GUARD" status
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'skill:autospec-run.*stale'
    printf '%s\n' "$output" | grep -q 'skill:autospec-explore.*live'
}
