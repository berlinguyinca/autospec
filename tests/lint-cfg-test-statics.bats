#!/usr/bin/env bats
# tests/lint-cfg-test-statics.bats
#
# Exercises scripts/lint-cfg-test-statics.sh against populated positive and
# negative cases (issue #3951): process-global `#[cfg(test)]` mutable statics
# in the executor_bridge tree must fail; thread_local! statics, `Mutex<()>`
# unit locks, constants, and reasoned waivers must pass.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-cfg-test-statics.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-cfg-test-statics"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"CFG_TEST_STATIC"* ]]
}

@test "cfg(test) AtomicU8 static is flagged" {
  run bash "$LINT" "$FIX/pos1-atomic.rs"
  [ "$status" -eq 1 ]
  [[ "$output" == *"CFG_TEST_STATIC:"* ]]
  [[ "$output" == *"PROBE"* ]]
  [[ "$output" == *"AtomicU8"* ]]
}

@test "cfg(test) Mutex<Vec> collector static is flagged" {
  run bash "$LINT" "$FIX/pos2-mutex.rs"
  [ "$status" -eq 1 ]
  [[ "$output" == *"CFG_TEST_STATIC:"* ]]
  [[ "$output" == *"COLLECTOR"* ]]
  [[ "$output" == *"Mutex<Vec<u32>>"* ]]
}

@test "path-qualified RefCell behind a comment line is flagged" {
  run bash "$LINT" "$FIX/pos3-stacked-attributes.rs"
  [ "$status" -eq 1 ]
  [[ "$output" == *"CFG_TEST_STATIC:"* ]]
  [[ "$output" == *"RACE"* ]]
}

@test "bare waiver marker does not suppress the finding" {
  run bash "$LINT" "$FIX/pos4-waiver-bare.rs"
  [ "$status" -eq 1 ]
  [[ "$output" == *"CFG_TEST_STATIC:"* ]]
  [[ "$output" != *"INFO:CFG_TEST_STATIC"* ]]
}

@test "thread_local! statics are exempt" {
  run bash "$LINT" "$FIX/neg1-thread-local.rs"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "Mutex(()) unit lock is exempt with an audit line" {
  run bash "$LINT" "$FIX/neg2-unit-mutex.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:CFG_TEST_STATIC:"* ]]
  [[ "$output" == *"FORK_LIFECYCLE"* ]]
  [[ "$output" == *"exempt (Mutex<()>)"* ]]
}

@test "constants without interior mutability pass" {
  run bash "$LINT" "$FIX/neg3-constants.rs"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "waiver with reason passes with an audit line" {
  run bash "$LINT" "$FIX/neg4-waiver.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:CFG_TEST_STATIC:"* ]]
  [[ "$output" == *"waived (Atomic)"* ]]
}

@test "default scan covers the executor_bridge tree and passes" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
  ! printf '%s\n' "$output" | grep -q '^CFG_TEST_STATIC:'
}

@test "no remaining process-global cfg(test) mutable statics in executor_bridge" {
  BRIDGE="$REPO_ROOT/crates/autospec-cli/src/commands/autonomous/executor_bridge"
  run bash "$LINT" "$BRIDGE.rs"
  [ "$status" -eq 0 ]
  run bash "$LINT" "$BRIDGE"
  [ "$status" -eq 0 ]
  ! printf '%s\n' "$output" | grep -q '^CFG_TEST_STATIC:'
}
