#!/usr/bin/env bats
# tests/loop/test_loop_check.bats — loop-check.sh CHECK exit normalization.
#
# Covers (docs/specs/2026-06-02-autospec-loop-design.md §Phase 2):
#   - exit 0 from CHECK -> echo "met", exit 0
#   - non-zero from CHECK -> echo "not-met", exit 1
#   - missing/empty CHECK command -> echo "not-met", exit 1
#   - CHECK command that does not exist -> echo "not-met", exit 1

ROOT="${BATS_TEST_DIRNAME}/../.."
LOOP_CHECK="$ROOT/skills/autospec-loop/scripts/loop-check.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export PATH="$TEST_TMP/bin:$PATH"
    mkdir -p "$TEST_TMP/bin"
}

teardown() { rm -rf "$TEST_TMP"; }

# ── exit 0 (met) ──────────────────────────────────────────────────────────────

@test "CHECK exit 0 -> output 'met' and exit 0" {
    run bash "$LOOP_CHECK" "true"
    [ "$status" -eq 0 ]
    [ "$output" = "met" ]
}

@test "CHECK 'test -e /dev/null' exits 0 -> met" {
    run bash "$LOOP_CHECK" "test -e /dev/null"
    [ "$status" -eq 0 ]
    [ "$output" = "met" ]
}

@test "CHECK inline shell script exiting 0 -> met" {
    run bash "$LOOP_CHECK" "bash -c 'exit 0'"
    [ "$status" -eq 0 ]
    [ "$output" = "met" ]
}

# ── non-zero (not-met) ────────────────────────────────────────────────────────

@test "CHECK exit 1 -> output 'not-met' and exit 1" {
    run bash "$LOOP_CHECK" "false"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

@test "CHECK exit 2 -> output 'not-met' and exit 1 (normalized)" {
    run bash "$LOOP_CHECK" "bash -c 'exit 2'"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

@test "CHECK exit 42 -> output 'not-met' and exit 1 (normalized)" {
    run bash "$LOOP_CHECK" "bash -c 'exit 42'"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

# ── missing / empty CHECK ────────────────────────────────────────────────────

@test "no CHECK argument -> not-met and exit 1" {
    run bash "$LOOP_CHECK"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

@test "empty string CHECK -> not-met and exit 1" {
    run bash "$LOOP_CHECK" ""
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

@test "literal 'null' CHECK -> not-met and exit 1" {
    run bash "$LOOP_CHECK" "null"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}

# ── nonexistent command ───────────────────────────────────────────────────────

@test "CHECK command not on PATH -> not-met and exit 1" {
    run bash "$LOOP_CHECK" "__no_such_cmd_xyz__"
    [ "$status" -eq 1 ]
    [ "$output" = "not-met" ]
}
