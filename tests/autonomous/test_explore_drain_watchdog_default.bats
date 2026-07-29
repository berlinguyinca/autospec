#!/usr/bin/env bats

@test "autonomous explore watchdog defaults to ten minutes" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS:-600' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}

@test "autonomous explore watchdog remains configurable" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
