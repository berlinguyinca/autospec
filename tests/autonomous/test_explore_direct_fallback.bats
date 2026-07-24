#!/usr/bin/env bats

@test "explore drain directly runs verifier when harness skipped it" {
  grep -q 'AUTOSPEC_EXPLORE_VERIFY_CMD_not_executed' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'bash "\$SCRIPT_DIR/autospec-explore.sh" --once' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
