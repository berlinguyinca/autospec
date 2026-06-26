#!/usr/bin/env bats
# tests/explore-on-drain.bats — TDD coverage for scripts/explore-on-drain.sh
#
# Tests run in isolation: HOME is redirected to a temp dir so the flag and
# counter files never touch the real ~/.autospec.
#
# External boundary stub: autospec-autonomy-gate.sh is placed on PATH as a
# fake so we control its exit code without touching the real gate.
#
# Per-repo scoping: AUTOSPEC_REPO is set to a known value so the slug-based
# subdirectory is deterministic without calling gh or git.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
SUT="$REPO_ROOT/scripts/explore-on-drain.sh"
STUB_DIR=""

# Canonical slug for the default test repo (owner/name → owner__name).
TEST_REPO="testowner/testrepo"
TEST_SLUG="testowner__testrepo"

setup() {
    STUB_DIR="$(mktemp -d)"
    export HOME="${BATS_TMPDIR}/home-$$"
    mkdir -p "${HOME}/.autospec"

    # Default gate stub: exit 0 (gate OK)
    cat > "${STUB_DIR}/autospec-autonomy-gate.sh" <<'GATE'
#!/usr/bin/env bash
exit 0
GATE
    chmod +x "${STUB_DIR}/autospec-autonomy-gate.sh"
}

teardown() {
    rm -rf "${STUB_DIR}" "${HOME}"
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

slug_dir() {
    printf '%s/.autospec/explore-on-drain/%s' "$HOME" "${1:-$TEST_SLUG}"
}

# ---------------------------------------------------------------------------
# Case 1: flag absent → stop (default unchanged behavior)
# ---------------------------------------------------------------------------

@test "flag absent: prints 'stop' and exits 0" {
    # No flag file created
    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]
}

# ---------------------------------------------------------------------------
# Case 2: flag present + gate OK + no dry-well + under cap → chain
# ---------------------------------------------------------------------------

@test "flag present, gate OK, under cap: prints 'chain' and increments per-repo counter" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    # No cycles file → defaults to 0 (under cap of 3)

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    # Counter must now be 1 under the slug-scoped subdirectory.
    cycles_file="$(slug_dir)/cycles"
    [ -f "$cycles_file" ]
    cycles="$(cat "$cycles_file")"
    [ "$cycles" -eq 1 ]
}

@test "flag present, gate OK, cycles at 2 (under cap 3): prints 'chain' and counter goes to 3" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "2" > "$(slug_dir)/cycles"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    cycles="$(cat "$(slug_dir)/cycles")"
    [ "$cycles" -eq 3 ]
}

# ---------------------------------------------------------------------------
# Case 3: at cap → stop, counter does not increment past cap
# ---------------------------------------------------------------------------

@test "flag present, gate OK, cycles at cap (3): prints 'stop' without incrementing" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "3" > "$(slug_dir)/cycles"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]

    # Counter must NOT have been incremented past cap
    cycles="$(cat "$(slug_dir)/cycles")"
    [ "$cycles" -eq 3 ]
}

@test "AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES=2: at cap=2 prints 'stop'" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "2" > "$(slug_dir)/cycles"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" \
        AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES=2 bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]
}

# ---------------------------------------------------------------------------
# Case 4: gate exit 1 → stop (even with flag present, under cap)
# ---------------------------------------------------------------------------

@test "gate exit 1: prints 'stop' even when flag present and under cap" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    # cycles file absent → 0

    # Override gate stub to exit 1
    cat > "${STUB_DIR}/autospec-autonomy-gate.sh" <<'GATE'
#!/usr/bin/env bash
exit 1
GATE
    chmod +x "${STUB_DIR}/autospec-autonomy-gate.sh"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]

    # Counter must NOT have been incremented (slug dir may not even exist)
    [ ! -f "$(slug_dir)/cycles" ]
}

# ---------------------------------------------------------------------------
# Case 5: dry-well guard — stop when previous explore cycle shipped 0 PRs
# ---------------------------------------------------------------------------

@test "dry-well: last-shipped=0 prints 'stop' even under cap with flag present" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "0" > "$(slug_dir)/last-shipped"
    # Counter is at 1 (under cap of 3) — would normally chain.
    echo "1" > "$(slug_dir)/cycles"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]

    # Counter must NOT have been incremented (dry-well fired before cap check).
    cycles="$(cat "$(slug_dir)/cycles")"
    [ "$cycles" -eq 1 ]
}

@test "dry-well: last-shipped=5 (non-zero) does not block chaining" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "5" > "$(slug_dir)/last-shipped"
    # Counter starts at 0 (under cap).

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]
}

@test "dry-well: corrupted last-shipped (non-numeric) does not block chaining" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"
    echo "bad-data" > "$(slug_dir)/last-shipped"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]
}

@test "dry-well: absent last-shipped file (first run) does not block chaining" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    # No last-shipped file → first run, no dry-well sentinel yet.

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]
}

# ---------------------------------------------------------------------------
# Case 6: per-repo scoping — counters are isolated by repo slug
# ---------------------------------------------------------------------------

@test "per-repo scoping: counter for repo-A does not affect repo-B" {
    touch "${HOME}/.autospec/explore-on-drain.flag"

    REPO_A="ownerA/repoA"
    REPO_B="ownerB/repoB"
    SLUG_A="ownerA__repoA"
    SLUG_B="ownerB__repoB"

    # Exhaust the cap for repo-A.
    mkdir -p "${HOME}/.autospec/explore-on-drain/${SLUG_A}"
    echo "3" > "${HOME}/.autospec/explore-on-drain/${SLUG_A}/cycles"

    # repo-B should still be able to chain (no cycles file).
    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$REPO_B" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    # repo-A is still at cap → stop.
    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$REPO_A" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]
}

@test "per-repo scoping: cycles file lives under slug subdir, not flat ~/.autospec" {
    touch "${HOME}/.autospec/explore-on-drain.flag"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    # File must be under the slug-scoped subdir.
    [ -f "$(slug_dir)/cycles" ]
    # The old flat path must NOT exist.
    [ ! -f "${HOME}/.autospec/explore-on-drain.cycles" ]
}

# ---------------------------------------------------------------------------
# Case 7: counter reset — removing the cycles file resets behavior
# (simulates the run-start rm -f that the orchestrator performs)
# ---------------------------------------------------------------------------

@test "counter reset: removing cycles file allows chaining after cap was reached" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    mkdir -p "$(slug_dir)"

    # Exhaust cap.
    echo "3" > "$(slug_dir)/cycles"
    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$output" = "stop" ]

    # Simulate run-start reset.
    rm -f "$(slug_dir)/cycles"

    # Now should chain again.
    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_REPO="$TEST_REPO" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    # Counter incremented to 1 post-reset.
    cycles="$(cat "$(slug_dir)/cycles")"
    [ "$cycles" -eq 1 ]
}
