#!/usr/bin/env bats

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "runtime policy classifier fixtures pass" {
  cd "$REPO_ROOT"
  cargo test runtime_policy
}
