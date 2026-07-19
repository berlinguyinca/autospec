#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "autonomous stop boundary regression is installed" {
  grep -q '_autospec_conductor_repo_stop_flag_path' "$REPO_ROOT/scripts/lib/autospec-loop.sh"
  grep -q 'repo-scoped immediate stop wins before Tier-1 queue scan' \
    "$REPO_ROOT/tests/autospec/test_conductor_wiring.bats"
}
