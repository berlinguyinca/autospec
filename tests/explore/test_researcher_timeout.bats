#!/usr/bin/env bats

@test "research cycle bounds each researcher and preserves the failure marker" {
  grep -q 'AUTOSPEC_RESEARCHER_TIMEOUT_SECS' \
    "$BATS_TEST_DIRNAME/../../scripts/explore-research-cycle.sh"
  grep -q 'kill-after=5' \
    "$BATS_TEST_DIRNAME/../../scripts/explore-research-cycle.sh"
  grep -q 'researcher_failed' \
    "$BATS_TEST_DIRNAME/../../scripts/explore-research-cycle.sh"
}
