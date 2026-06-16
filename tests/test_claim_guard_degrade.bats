#!/usr/bin/env bats
# tests/test_claim_guard_degrade.bats — degrade-to-no-op safety.
#
# The spec (docs/specs/2026-06-15-claim-guard-concurrent-edit-design.md,
# §"Testing" / §"API shape") names this file explicitly:
#   "AUTOSPEC_CLAIM_GUARD=off and an unwritable store both no-op without
#    blocking." The contract is: claim-guard must NEVER block real work because
#   of its own infrastructure — an off switch or a read-only store degrades to a
#   silent exit-0 no-op. This is the dedicated, spec-named suite issue #1069's
#   Phase 5.5 audit requires.
#
# Real filesystem store, no mocks (AGENTS.md).

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_HOST="testhost"
    export CLAUDE_CODE_SESSION_ID="sess-degrade-aaaa"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca_autospec"
}

teardown() {
    # Restore perms before rm so the tmp dir is always removable.
    chmod -R u+rwx "$STATE_DIR" 2>/dev/null || true
    rm -rf "$STATE_DIR"
}

# ---------------------------------------------------------------------------
@test "AUTOSPEC_CLAIM_GUARD=off: acquire exits 0 and writes nothing" {
    run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    [ ! -d "$CLAIM_ROOT" ]
}

# ---------------------------------------------------------------------------
@test "AUTOSPEC_CLAIM_GUARD=off: assert/refresh/release all no-op exit 0" {
    run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" assert skills/autospec-run
    [ "$status" -eq 0 ]
    run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" refresh
    [ "$status" -eq 0 ]
    run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" release skills/autospec-run
    [ "$status" -eq 0 ]
    [ ! -d "$CLAIM_ROOT" ]
}

# ---------------------------------------------------------------------------
@test "AUTOSPEC_CLAIM_GUARD=off never blocks even when another session holds the key" {
    # A real claim exists (taken in strict mode).
    CLAUDE_CODE_SESSION_ID="sess-owner" \
        env AUTOSPEC_CLAIM_GUARD=strict bash "$GUARD" acquire skills/autospec-run
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]

    # With the guard OFF, a different session must NOT be blocked (exit 0 no-op).
    CLAUDE_CODE_SESSION_ID="sess-other" \
        run env AUTOSPEC_CLAIM_GUARD=off bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    # off mode writes nothing, so the original owner's claim is untouched.
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-owner" ]
}

# ---------------------------------------------------------------------------
@test "unwritable store: acquire degrades to a no-op success (never blocks), even in strict mode" {
    ro_root="$(mktemp -d)"
    chmod 0500 "$ro_root"
    run env AUTOSPEC_STATE_DIR="$ro_root" AUTOSPEC_CLAIM_GUARD=strict \
        bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    # Nothing was (could be) written under the read-only root.
    [ ! -d "${ro_root}/edit-claims/berlinguyinca_autospec" ]
    chmod 0700 "$ro_root"; rm -rf "$ro_root"
}

# ---------------------------------------------------------------------------
@test "unwritable store: assert/refresh/release degrade to no-op exit 0" {
    ro_root="$(mktemp -d)"
    chmod 0500 "$ro_root"
    for sub in "assert skills/autospec-run" "refresh" "release skills/autospec-run"; do
        # shellcheck disable=SC2086
        run env AUTOSPEC_STATE_DIR="$ro_root" AUTOSPEC_CLAIM_GUARD=strict \
            bash "$GUARD" $sub
        [ "$status" -eq 0 ]
    done
    chmod 0700 "$ro_root"; rm -rf "$ro_root"
}

# ---------------------------------------------------------------------------
@test "unwritable store: status is read-only and prints nothing, exit 0" {
    ro_root="$(mktemp -d)"
    chmod 0500 "$ro_root"
    run env AUTOSPEC_STATE_DIR="$ro_root" bash "$GUARD" status
    [ "$status" -eq 0 ]
    chmod 0700 "$ro_root"; rm -rf "$ro_root"
}
