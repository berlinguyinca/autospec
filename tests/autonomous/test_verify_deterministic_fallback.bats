#!/usr/bin/env bats

@test "verifier exposes deterministic fallback for bounded evidence" {
  grep -q 'deterministic_fallback' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-verify-drain.sh"
  grep -q 'AUTOSPEC_AUTONOMOUS_DETERMINISTIC_VERIFY' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-verify-drain.sh"
}

@test "verifier fallback requires an existing path and line" {
  grep -q 'os.path.isfile(path)' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-verify-drain.sh"
  grep -q 're.search' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-verify-drain.sh"
}
