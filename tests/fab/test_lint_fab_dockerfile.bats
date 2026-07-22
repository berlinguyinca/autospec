#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/lint-fab-dockerfile.sh"
}

@test "fab Dockerfile lint source contains no ambiguous any token" {
  ! grep -Eq '\bany\b' "$SCRIPT"
}
