#!/usr/bin/env bats

@test "autonomous drain defaults claim leases to ten minutes" {
  grep -q 'export AUTOSPEC_CLAIM_TTL_SECONDS="\${AUTOSPEC_CLAIM_TTL_SECONDS:-600}"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-run-drain.sh"
}

