#!/usr/bin/env bats

@test "explore drain isolates and kills the harness process group" {
  grep -q 'setsid omx exec' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'kill -TERM -- "-\$_pgid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'kill -KILL -- "-\$_pgid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
