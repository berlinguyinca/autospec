#!/usr/bin/env bats

@test "nested explore drain returns a suppressed dry result" {
  run env AUTOSPEC_EXPLORE_DRAIN_ACTIVE=1 \
    bash "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh" --once
  [ "$status" -eq 0 ]
  [[ "$output" == *'nested-explore-suppressed'* ]]
}

@test "explore drain exports recursion marker before harness launch" {
  grep -q 'export AUTOSPEC_EXPLORE_DRAIN_ACTIVE=1' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
