#!/usr/bin/env bats
# Vacuous test fixture — always passes regardless of behavior.
# Used to verify that mutants SURVIVE against a bad test suite.

setup() {
    FIXTURE_DIR="$(cd "$BATS_TEST_DIRNAME" && pwd)"
    SOURCE="$FIXTURE_DIR/sample.sh"
}

@test "check_equal: vacuous assertion always passes" {
    # This test is vacuous — it doesn't actually verify check_equal's behavior.
    # A mutant that flips == to != will survive because this test never catches it.
    source "$SOURCE"
    run check_equal "anything"
    # Bad: || true makes the assertion a no-op
    [ "$status" -eq 0 ] || true
}
