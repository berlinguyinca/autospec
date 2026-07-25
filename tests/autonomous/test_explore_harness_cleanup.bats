#!/usr/bin/env bats

@test "explore drain isolates and kills the harness process group" {
  grep -q 'setsid omx exec' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'lib/autospec-process-tree.sh' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'autospec_kill_tree "\$child_pid" separate' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 120' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
