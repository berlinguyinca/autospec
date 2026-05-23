#!/usr/bin/env bats
# Deliberate vacuous test fixture — M1 (VACUOUS_GREP_INVERSE_OR_TRUE) should catch this.
# Used by integration test 6 in tests/mutation-integration.bats.

@test "always passes due to vacuous grep" {
    grep -qv "X" /dev/null || true
    run echo "ok"
    [ "$status" -eq 0 ]
}
