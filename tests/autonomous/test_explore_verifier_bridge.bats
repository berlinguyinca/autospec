#!/usr/bin/env bats

@test "explore drain embeds verifier command across harness boundary" {
  grep -q 'export AUTOSPEC_EXPLORE_VERIFY_CMD="\$VERIFY_CMD"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'setsid env AUTOSPEC_EXPLORE_VERIFY_CMD="\$VERIFY_CMD"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
