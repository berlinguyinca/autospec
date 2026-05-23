#!/usr/bin/env bats
# Deliberate zero-assertion bats fixture for Python density-floor integration test.
# This test has no assert/expect/run/grep — density floor should flag it.
# Used by integration test 8 in tests/mutation-integration.bats.

@test "python check always returns zero" {
    python3 -c "print(0)"
}
