#!/usr/bin/env bats
# tests/explore-on-drain.bats — TDD coverage for scripts/explore-on-drain.sh
#
# Tests run in isolation: HOME is redirected to a temp dir so the flag and
# counter files never touch the real ~/.autospec.
#
# External boundary stub: autospec-autonomy-gate.sh is placed on PATH as a
# fake so we control its exit code without touching the real gate.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
SUT="$REPO_ROOT/scripts/explore-on-drain.sh"
STUB_DIR=""

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
# Case 1: flag absent → stop (default unchanged behavior)
# ---------------------------------------------------------------------------

@test "flag absent: prints 'stop' and exits 0" {
    # No flag file created
    run env PATH="${STUB_DIR}:${PATH}" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]
}

# ---------------------------------------------------------------------------
# Case 2: flag present + gate OK + under cap → chain, counter incremented
# ---------------------------------------------------------------------------

@test "flag present, gate OK, under cap: prints 'chain' and increments counter" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    # No cycles file → defaults to 0 (under cap of 3)

    run env PATH="${STUB_DIR}:${PATH}" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    # Counter must now be 1
    cycles="$(cat "${HOME}/.autospec/explore-on-drain.cycles")"
    [ "$cycles" -eq 1 ]
}

@test "flag present, gate OK, cycles at 2 (under cap 3): prints 'chain' and counter goes to 3" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    echo "2" > "${HOME}/.autospec/explore-on-drain.cycles"

    run env PATH="${STUB_DIR}:${PATH}" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "chain" ]

    cycles="$(cat "${HOME}/.autospec/explore-on-drain.cycles")"
    [ "$cycles" -eq 3 ]
}

# ---------------------------------------------------------------------------
# Case 3: at cap → stop, counter does not increment past cap
# ---------------------------------------------------------------------------

@test "flag present, gate OK, cycles at cap (3): prints 'stop' without incrementing" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    echo "3" > "${HOME}/.autospec/explore-on-drain.cycles"

    run env PATH="${STUB_DIR}:${PATH}" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]

    # Counter must NOT have been incremented past cap
    cycles="$(cat "${HOME}/.autospec/explore-on-drain.cycles")"
    [ "$cycles" -eq 3 ]
}

@test "AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES=2: at cap=2 prints 'stop'" {
    touch "${HOME}/.autospec/explore-on-drain.flag"
    echo "2" > "${HOME}/.autospec/explore-on-drain.cycles"

    run env PATH="${STUB_DIR}:${PATH}" AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES=2 bash "$SUT"
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

    run env PATH="${STUB_DIR}:${PATH}" bash "$SUT"
    [ "$status" -eq 0 ]
    [ "$output" = "stop" ]

    # Counter must NOT have been incremented
    [ ! -f "${HOME}/.autospec/explore-on-drain.cycles" ]
}
