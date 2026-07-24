#!/usr/bin/env bats

@test "explore drain has an absolute runtime cap" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS:-900' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'max runtime' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}

@test "explore stall default remains six minutes with runtime config" {
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 600' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
