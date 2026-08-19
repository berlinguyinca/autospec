#!/usr/bin/env bats

@test "explore drain isolates and kills the harness process group" {
  grep -q 'run_in_new_session "\$HARNESS_DISPATCHER" exec' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'kill -TERM -- "-\$_pgid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'kill -KILL -- "-\$_pgid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 120' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
