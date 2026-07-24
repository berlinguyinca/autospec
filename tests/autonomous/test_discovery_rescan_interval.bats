#!/usr/bin/env bats

@test "autonomous discovery defaults to a five-minute rescan" {
  grep -q 'AUTOSPEC_RESCAN_INTERVAL:-300' \
    "$BATS_TEST_DIRNAME/../../scripts/lib/autospec-loop.sh"
}

@test "autonomous discovery keeps the interval configurable" {
  grep -q 'AUTOSPEC_RESCAN_INTERVAL' \
    "$BATS_TEST_DIRNAME/../../scripts/lib/autospec-loop.sh"
}
