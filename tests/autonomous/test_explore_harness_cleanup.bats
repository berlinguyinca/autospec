#!/usr/bin/env bats

@test "explore drain isolates and kills the harness process group" {
  reaper="$BATS_TEST_DIRNAME/../../scripts/lib/process-tree.sh"
  grep -q 'setsid omx exec' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'kill -TERM -- "-\$pgid"' "$reaper"
  grep -q 'kill -KILL -- "-\$pgid"' "$reaper"
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 120' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
