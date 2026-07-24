#!/usr/bin/env bats

@test "explore safety filing resolves repository and installed autospec binaries" {
  grep -q 'target", "debug", "autospec"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-explore.sh"
  grep -q '~/.autospec/bin/autospec' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-explore.sh"
  grep -q 'shutil.which("autospec")' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-explore.sh"
}
