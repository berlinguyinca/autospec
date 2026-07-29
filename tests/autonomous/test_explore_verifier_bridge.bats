#!/usr/bin/env bats

@test "explore drain embeds verifier command across harness boundary" {
  grep -q 'AUTOSPEC_EXPLORE_VERIFY_CMD=%q' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'SKILL_INVOCATION="\$VERIFY_ASSIGNMENT' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
}
