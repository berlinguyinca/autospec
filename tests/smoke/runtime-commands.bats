#!/usr/bin/env bats

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "runtime classify command fixtures pass" {
  cd "$REPO_ROOT"
  cargo test runtime_commands
}
