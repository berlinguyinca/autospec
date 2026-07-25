#!/usr/bin/env bats

@test "autonomous explore watchdog defaults to two minutes" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS:-120' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}

@test "autonomous explore watchdog remains configurable" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
