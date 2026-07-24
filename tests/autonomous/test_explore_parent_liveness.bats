#!/usr/bin/env bats

@test "explore drain exports its owner PID" {
  grep -q 'export AUTOSPEC_EXPLORE_PARENT_PID="\$\$"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}

@test "explore script installs a parent liveness watcher" {
  grep -q 'AUTOSPEC_EXPLORE_PARENT_PID' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-explore.sh"
  grep -q 'kill -TERM "\$\$"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-explore.sh"
}
