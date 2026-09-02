#!/usr/bin/env bats
# tests/unit/proxy-direct-borrow-lifetime.bats — regression coverage for #3475.
#
# `entry.file_name()` returns an owned OsString. Binding
# `let name = entry.file_name().to_string_lossy();` borrows a temporary that is
# freed at the end of the statement, which is E0716 and stops the whole
# `autospec-cli` test target from COMPILING — every #[cfg(test)] unit test under
# crates/autospec-cli/src/** becomes unrunnable rather than merely failing.
#
# The regression signal for this defect is therefore compilation, not assertion.
# Real compiler, no mocks: this runs cargo against the actual crate.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "autospec-cli test targets typecheck (E0716 borrow-lifetime guard)" {
  if ! command -v cargo >/dev/null 2>&1; then
    skip "cargo not installed"
  fi
  run cargo check --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --tests --message-format short
  [ "$status" -eq 0 ]
  [[ "$output" != *"E0716"* ]]
}

@test "proxy_direct archive checks bind the OsString before borrowing" {
  local file="$REPO_ROOT/crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/proxy_direct.rs"
  [ -f "$file" ]
  # The defect shape: to_string_lossy() applied directly to the file_name()
  # temporary in a let binding. Zero occurrences expected.
  run grep -c 'let [a-z_]* = [a-z_]*\.file_name()\.to_string_lossy()' "$file"
  [ "$output" = "0" ]
}
