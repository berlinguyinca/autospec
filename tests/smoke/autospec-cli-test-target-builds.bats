#!/usr/bin/env bats
# tests/smoke/autospec-cli-test-target-builds.bats — smoke coverage for #3475.
#
# The issue's Primary smoke test. Builds every autospec-cli test binary without
# running them: this is what regressed, and it is what a borrow-lifetime defect
# in any #[cfg(test)] module would break again. Distinct from the unit tier,
# which only typechecks -- this links the actual test executables, including
# `bin "autospec" test`, whose unit tests were unreachable while #3475 was open.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "cargo test -p autospec-cli --no-run builds all test binaries" {
  if ! command -v cargo >/dev/null 2>&1; then
    skip "cargo not installed"
  fi
  run cargo test --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --no-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"Executable unittests src/main.rs"* ]]
}
