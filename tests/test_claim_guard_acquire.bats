#!/usr/bin/env bats
# tests/test_claim_guard_acquire.bats — acquire/release round-trip + status.
#
# Covers AC: acquire writes a JSON claim under edit-claims/<repo-slug>/,
# status lists it, release removes it, skill-path resolution to skill:<name>,
# AUTOSPEC_CLAIM_GUARD=off degrades to a no-op, and assert sees a free key.
#
# Real filesystem store, no mocks (AGENTS.md). The store root is redirected via
# AUTOSPEC_STATE_DIR to a per-test tmp dir so we never touch ~/.autospec.

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    # Deterministic repo identity so the slug is stable and assertable without
    # re-deriving it from the SUT (self-consistent-fixture trap).
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_CLAIM_GUARD="strict"
    export CLAUDE_CODE_SESSION_ID="sess-acquire-aaaa"
    export AUTOSPEC_HOST="testhost"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca_autospec"
}

teardown() {
    rm -rf "$STATE_DIR"
}

# ---------------------------------------------------------------------------
@test "claim-guard.sh is executable and bash-valid" {
    [ -x "$GUARD" ]
    run bash -n "$GUARD"
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "acquire a skill path writes a skill:<name> JSON claim under edit-claims/<repo-slug>/" {
    run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    # Lock key for a skills/<name>/... path is skill:<name>; the file name
    # encodes the key with ':' slugified to '-'.
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -f "$claim_file" ]

    run jq -r '.lock_key' "$claim_file"
    [ "$status" -eq 0 ]
    [ "$output" = "skill:autospec-run" ]

    run jq -r '.owner_session' "$claim_file"
    [ "$output" = "sess-acquire-aaaa" ]

    run jq -r '.host' "$claim_file"
    [ "$output" = "testhost" ]

    run jq -r '.ttl_seconds' "$claim_file"
    [ "$status" -eq 0 ]
    [ "$output" -gt 0 ]
}

# ---------------------------------------------------------------------------
@test "a deeper skill subpath and a skill golden both resolve to skill:<name>" {
    run bash "$GUARD" acquire "skills/autospec-run/scripts/heartbeat-write.sh"
    [ "$status" -eq 0 ]
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]

    bash "$GUARD" release
    run bash "$GUARD" acquire "tests/fixtures/skill-goldens/autospec-run.sha256"
    [ "$status" -eq 0 ]
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
}

# ---------------------------------------------------------------------------
@test "a non-skill path resolves to a path:<glob> claim" {
    run bash "$GUARD" acquire "scripts/validate.sh"
    [ "$status" -eq 0 ]
    # exactly one claim json exists, and its key is path:...
    claim_file="$(ls "${CLAIM_ROOT}"/*.json)"
    run jq -r '.lock_key' "$claim_file"
    [ "$status" -eq 0 ]
    case "$output" in
        path:*) : ;;
        *) printf 'expected path: key, got %s\n' "$output" >&2; return 1 ;;
    esac
}

# ---------------------------------------------------------------------------
@test "status lists a live claim after acquire and is empty after release" {
    bash "$GUARD" acquire skills/autospec-run

    run bash "$GUARD" status
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "skill:autospec-run"

    run bash "$GUARD" release
    [ "$status" -eq 0 ]
    [ ! -f "${CLAIM_ROOT}/skill-autospec-run.json" ]

    run bash "$GUARD" status
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q "skill:autospec-run"
}

# ---------------------------------------------------------------------------
@test "release with explicit keys drops only those claims" {
    bash "$GUARD" acquire skills/autospec-run skills/autospec-explore
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
    [ -f "${CLAIM_ROOT}/skill-autospec-explore.json" ]

    run bash "$GUARD" release skills/autospec-run
    [ "$status" -eq 0 ]
    [ ! -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
    [ -f "${CLAIM_ROOT}/skill-autospec-explore.json" ]
}

# ---------------------------------------------------------------------------
@test "re-acquire of my own live claim is an idempotent success" {
    run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "assert on a free key exits 0; on my own key exits 0" {
    run bash "$GUARD" assert skills/autospec-run
    [ "$status" -eq 0 ]
    bash "$GUARD" acquire skills/autospec-run
    run bash "$GUARD" assert skills/autospec-run
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "refresh bumps updated_at on my claim" {
    bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    before="$(jq -r '.updated_at' "$claim_file")"
    # force a measurable time gap by rewriting updated_at into the past
    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"
    run bash "$GUARD" refresh
    [ "$status" -eq 0 ]
    after="$(jq -r '.updated_at' "$claim_file")"
    [ "$after" != "2000-01-01T00:00:00Z" ]
}

# ---------------------------------------------------------------------------
@test "AUTOSPEC_CLAIM_GUARD=off acquire exits 0 and writes no claim" {
    run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    [ ! -d "${CLAIM_ROOT}" ] || [ -z "$(ls -A "${CLAIM_ROOT}" 2>/dev/null)" ]
}

# ---------------------------------------------------------------------------
@test "unwritable store degrades to a no-op success (never blocks)" {
    # Point the state dir at a location we cannot create under.
    ro_root="$(mktemp -d)"
    chmod 0500 "$ro_root"
    run env AUTOSPEC_STATE_DIR="${ro_root}/sub" AUTOSPEC_CLAIM_GUARD=strict \
        bash "$GUARD" acquire skills/autospec-run
    chmod 0700 "$ro_root"; rm -rf "$ro_root"
    [ "$status" -eq 0 ]
}
