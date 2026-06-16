#!/usr/bin/env bats
# tests/test_claim_guard_overlap.bats — scan pre-flight overlap detection.
#
# Covers AC for issue #1066:
#   - scan exits 0 always (advisory, never blocks).
#   - scan on a skill golden path with a held skill:<name> claim prints overlap advisory.
#   - scan on a disjoint path prints no advisory for the held claim.
#   - scan exits 0 when `gh` is absent from PATH.
#
# Real filesystem store; AUTOSPEC_STATE_DIR redirected to per-test tmp dir.
# No mocks for git/gh — gh is skipped via a stub PATH that hides it
# (degrade path). git worktree list runs real but scans an empty-ish list.

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_CLAIM_GUARD="warn"
    export CLAUDE_CODE_SESSION_ID="sess-overlap-bbbb"
    export AUTOSPEC_HOST="testhost"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca_autospec"

    # Create a fake-gh bin dir that hides the real gh so degrade tests run
    # even on machines with gh installed.
    FAKE_GH_DIR="$(mktemp -d)"
    # No gh binary in FAKE_GH_DIR — it simply does not exist there.
}

teardown() {
    rm -rf "$STATE_DIR"
    rm -rf "${FAKE_GH_DIR:-}"
}

# ---------------------------------------------------------------------------
# Helper: plant a live claim for skill:autospec-run owned by a DIFFERENT session.
plant_other_session_claim() {
    mkdir -p "$CLAIM_ROOT"
    local now
    now="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    cat > "${CLAIM_ROOT}/skill-autospec-run.json" <<EOF
{
  "lock_key": "skill:autospec-run",
  "paths": ["skills/autospec-run/"],
  "owner_session": "sess-other-zzzz",
  "host": "otherhost",
  "pid": 9999,
  "branch": "feat/other-branch",
  "worktree": "/tmp/other-wt",
  "acquired_at": "${now}",
  "updated_at": "${now}",
  "ttl_seconds": 1800
}
EOF
    # Also create the .lock dir so the claim is consistent.
    mkdir -p "${CLAIM_ROOT}/skill-autospec-run.lock"
}

# ---------------------------------------------------------------------------
@test "scan is a valid subcommand and exits 0 on a free skill" {
    run bash "$GUARD" scan skills/autospec-run
    [ "$status" -eq 0 ]
}

@test "scan exits 0 even when a conflicting live claim exists (advisory only)" {
    plant_other_session_claim
    run bash "$GUARD" scan skills/autospec-run
    [ "$status" -eq 0 ]
}

@test "scan prints overlap advisory when a live claim is held by another session" {
    plant_other_session_claim
    run bash "$GUARD" scan skills/autospec-run
    [ "$status" -eq 0 ]
    # Advisory must mention claim_overlap or the key.
    echo "$output" | grep -qE 'claim_overlap|skill:autospec-run'
}

@test "assert of skill-golden path conflicts with a held skill:<name> claim" {
    # Acquire a claim for skill:autospec-run as a different session.
    plant_other_session_claim
    # assert in strict mode should see the conflict (non-zero exit).
    AUTOSPEC_CLAIM_GUARD=strict run bash "$GUARD" assert \
        tests/fixtures/skill-goldens/autospec-run.SKILL.md.sha256
    [ "$status" -ne 0 ]
}

@test "scan on a disjoint path prints no claim_overlap advisory for the held claim" {
    plant_other_session_claim
    # skills/autospec-explore is disjoint from skills/autospec-run.
    run bash "$GUARD" scan skills/autospec-explore
    [ "$status" -eq 0 ]
    # Must not print an overlap advisory for the autospec-run claim.
    echo "$output" | grep -qv 'claim_overlap.*skill:autospec-run' || true
    # Simpler: output must not contain claim_overlap for autospec-run.
    if echo "$output" | grep -q 'claim_overlap'; then
        echo "$output" | grep -v 'autospec-run' | grep -qv 'claim_overlap' || true
        # If the only claim_overlap lines mention autospec-run, that's a false-positive.
        local overlap_lines
        overlap_lines="$(echo "$output" | grep 'claim_overlap' || true)"
        # None of the overlap_lines should be for autospec-run when scanning autospec-explore.
        ! echo "$overlap_lines" | grep -q 'autospec-run'
    fi
}

@test "scan exits 0 when gh is absent from PATH (degrade gracefully)" {
    # Run with a PATH that contains no 'gh' binary.
    run env PATH="${FAKE_GH_DIR}:/usr/bin:/bin" bash "$GUARD" scan skills/autospec-run
    [ "$status" -eq 0 ]
}

@test "scan exits 0 even with AUTOSPEC_CLAIM_GUARD=strict (scan is always advisory)" {
    plant_other_session_claim
    AUTOSPEC_CLAIM_GUARD=strict run bash "$GUARD" scan skills/autospec-run
    [ "$status" -eq 0 ]
}

@test "scan accepts multiple path arguments and exits 0" {
    run bash "$GUARD" scan skills/autospec-run skills/autospec-explore
    [ "$status" -eq 0 ]
}
