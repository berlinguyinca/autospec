#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  AGENTS_FILE="${AGENTS_FILE:-$REPO_ROOT/AGENTS.md}"
}

@test "engineering standards name the Rust test runner" {
  grep -Fq 'Rust tests run with `cargo test`' "$AGENTS_FILE"
}

@test "engineering standards do not deny a language-level test runner" {
  run grep -F 'this repo has no language-level test runner' "$AGENTS_FILE"
  [ "$status" -eq 1 ]
}
