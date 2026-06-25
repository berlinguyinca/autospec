#!/usr/bin/env bats
# tests/autospec-repo-lock.bats — TDD tests for scripts/autospec-repo-lock.sh
#
# Covers:
#   1. Default-off (AUTOSPEC_REPO_LOCK unset) → acquire/release are no-ops.
#   2. AUTOSPEC_REPO_LOCK=1 → acquire creates lock dir; second acquire times out.
#   3. Release frees the lock so a subsequent acquire succeeds.
#   4. Two distinct slugs never contend with each other.
#   5. Script passes bash -n syntax check.

bats_require_minimum_version 1.5.0

LOCK_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/autospec-repo-lock.sh"
SLUG_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/repo-slug.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_REPO_LOCK_DIR="$TEST_TMP/repo-locks"
    # Point repo-slug at a known repo
    export _TEST_REPO="testorg/testrepo"
    # Unset the opt-in flag by default; individual tests override as needed
    unset AUTOSPEC_REPO_LOCK || true
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── Syntax ────────────────────────────────────────────────────────────────────

@test "autospec-repo-lock.sh passes bash -n syntax check" {
    run bash -n "$LOCK_SCRIPT"
    [ "$status" -eq 0 ]
}

# ── Default-off: AUTOSPEC_REPO_LOCK unset ────────────────────────────────────

@test "default-off: acquire exits 0 without AUTOSPEC_REPO_LOCK set" {
    unset AUTOSPEC_REPO_LOCK
    run bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ "$status" -eq 0 ]
}

@test "default-off: release exits 0 without AUTOSPEC_REPO_LOCK set" {
    unset AUTOSPEC_REPO_LOCK
    run bash "$LOCK_SCRIPT" release testorg__testrepo
    [ "$status" -eq 0 ]
}

@test "default-off: no lock dir created when AUTOSPEC_REPO_LOCK unset" {
    unset AUTOSPEC_REPO_LOCK
    bash "$LOCK_SCRIPT" acquire testorg__testrepo
    bash "$LOCK_SCRIPT" release testorg__testrepo
    # Lock dir should NOT have been created
    [ ! -d "$AUTOSPEC_REPO_LOCK_DIR" ]
}

# ── Enabled: AUTOSPEC_REPO_LOCK=1 ────────────────────────────────────────────

@test "enabled: acquire creates lock dir" {
    export AUTOSPEC_REPO_LOCK=1
    run bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ "$status" -eq 0 ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock" ]
}

@test "enabled: acquire records pid file inside lock dir" {
    export AUTOSPEC_REPO_LOCK=1
    bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ -f "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock/pid" ]
}

@test "enabled: release removes lock dir" {
    export AUTOSPEC_REPO_LOCK=1
    bash "$LOCK_SCRIPT" acquire testorg__testrepo
    bash "$LOCK_SCRIPT" release testorg__testrepo
    [ ! -d "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock" ]
}

@test "enabled: acquire then release then re-acquire succeeds" {
    export AUTOSPEC_REPO_LOCK=1
    bash "$LOCK_SCRIPT" acquire testorg__testrepo
    bash "$LOCK_SCRIPT" release testorg__testrepo
    run bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ "$status" -eq 0 ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock" ]
}

@test "enabled: second acquire on same slug exits non-zero after timeout" {
    export AUTOSPEC_REPO_LOCK=1
    export AUTOSPEC_REPO_LOCK_TIMEOUT=2   # short timeout for test speed
    export AUTOSPEC_REPO_LOCK_POLL=1      # 1-second poll interval
    # Plant a lock manually with a LIVE pid (the bats test process itself).
    # This simulates another same-machine process currently holding the lock.
    mkdir -p "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock"
    printf '%s\n' "$$" > "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock/pid"
    # Second acquire must time out and exit non-zero
    run bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ "$status" -ne 0 ]
}

@test "enabled: release by wrong slug is a no-op (does not remove another slug's lock)" {
    export AUTOSPEC_REPO_LOCK=1
    bash "$LOCK_SCRIPT" acquire testorg__testrepo
    # Release a different slug — should not touch the first slug's lock
    run bash "$LOCK_SCRIPT" release otherorg__otherrepo
    [ "$status" -eq 0 ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock" ]
}

# ── Two distinct slugs never contend ─────────────────────────────────────────

@test "enabled: two distinct slugs can both be acquired without contention" {
    export AUTOSPEC_REPO_LOCK=1
    run bash "$LOCK_SCRIPT" acquire slugA__repo
    [ "$status" -eq 0 ]
    run bash "$LOCK_SCRIPT" acquire slugB__repo
    [ "$status" -eq 0 ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/slugA__repo.lock" ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/slugB__repo.lock" ]
}

# ── Stale lock cleanup ────────────────────────────────────────────────────────

@test "enabled: stale lock (pid not running) is forcibly broken on acquire" {
    export AUTOSPEC_REPO_LOCK=1
    export AUTOSPEC_REPO_LOCK_TIMEOUT=2
    export AUTOSPEC_REPO_LOCK_POLL=1
    # Manually plant a stale lock with a dead PID
    mkdir -p "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock"
    # Use PID 99999 — very unlikely to be running; if it is, skip gracefully
    echo "99999" > "$AUTOSPEC_REPO_LOCK_DIR/testorg__testrepo.lock/pid"
    run bash "$LOCK_SCRIPT" acquire testorg__testrepo
    [ "$status" -eq 0 ]
}

# ── Canonical-slug integration (source repo-slug.sh) ─────────────────────────

@test "enabled: acquire works when slug is derived via repo-slug.sh canonical_slug" {
    export AUTOSPEC_REPO_LOCK=1
    slug="$(bash -c "source '$SLUG_SCRIPT'; canonical_slug 'testorg/testrepo'")"
    run bash "$LOCK_SCRIPT" acquire "$slug"
    [ "$status" -eq 0 ]
    [ -d "$AUTOSPEC_REPO_LOCK_DIR/${slug}.lock" ]
}
