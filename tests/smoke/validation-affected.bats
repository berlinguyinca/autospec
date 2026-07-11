#!/usr/bin/env bats

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "validation affected routing fixtures pass" {
  cd "$REPO_ROOT"
  cargo test validation_affected
}
