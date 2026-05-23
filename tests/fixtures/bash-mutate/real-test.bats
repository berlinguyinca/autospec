#!/usr/bin/env bats
# Real test fixture — properly verifies behavior, killing mutants.

setup() {
    FIXTURE_DIR="$(cd "$BATS_TEST_DIRNAME" && pwd)"
    SOURCE="$FIXTURE_DIR/sample.sh"
}

@test "check_equal: returns match for expected input" {
    source "$SOURCE"
    run check_equal "expected"
    [ "$status" -eq 0 ]
    [ "$output" = "match" ]
}

@test "check_equal: returns nothing for unexpected input" {
    source "$SOURCE"
    run check_equal "wrong"
    [ "$status" -eq 0 ]
    [ "$output" = "" ]
}

@test "assert_output: passes for match" {
    source "$SOURCE"
    run assert_output "match"
    [ "$status" -eq 0 ]
}

@test "assert_output: fails for non-match" {
    source "$SOURCE"
    run assert_output "wrong"
    [ "$status" -ne 0 ]
}
