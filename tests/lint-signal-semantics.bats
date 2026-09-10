#!/usr/bin/env bats
# tests/lint-signal-semantics.bats
#
# Exercises scripts/lint-signal-semantics.sh against populated positive and
# negative cases (issue #4088): a stderr discard to /dev/null followed by a
# counting reducer must fail; no-discard, no-reducer, and reduced-before-
# discard lines must pass; waived lines (reason mandatory) emit an advisory
# INFO line and pass.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-signal-semantics.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-signal-semantics"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"SIGNAL_SEMANTICS"* ]]
  [[ "$output" == *"wc"* ]]
}

@test "swallowed-stderr count (wc) is flagged" {
  run bash "$LINT" "$FIX/pos1-count.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SIGNAL_SEMANTICS:tests/fixtures/lint-signal-semantics/pos1-count.sh:4:"* ]]
  [[ "$output" == *"2>/dev/null | wc -l"* ]]
}

@test "swallowed-stderr grep -c is flagged" {
  run bash "$LINT" "$FIX/pos2-grep-c.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SIGNAL_SEMANTICS:"* ]]
  [[ "$output" == *"grep -cF"* ]]
}

@test "spaced redirect variant is flagged" {
  run bash "$LINT" "$FIX/pos3-spaced.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SIGNAL_SEMANTICS:"* ]]
}

@test "no stderr discard is not a finding" {
  run bash "$LINT" "$FIX/neg1-no-swallow.sh"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "no counting reducer is not a finding" {
  run bash "$LINT" "$FIX/neg2-no-reducer.sh"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "reducer before the discard is not a finding" {
  run bash "$LINT" "$FIX/neg3-reducer-first.sh"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "same-line waiver with reason passes with an INFO audit line" {
  run bash "$LINT" "$FIX/neg4-waived-same-line.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:SIGNAL_SEMANTICS:"* ]]
  [[ "$output" == *"waived:"* ]]
}

@test "line-above waiver with reason passes with an INFO audit line" {
  run bash "$LINT" "$FIX/neg5-waived-line-above.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:SIGNAL_SEMANTICS:"* ]]
}

@test "bare waiver marker is rejected and the line stays flagged" {
  run bash "$LINT" "$FIX/neg6-bare-waiver.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SIGNAL_SEMANTICS:tests/fixtures/lint-signal-semantics/neg6-bare-waiver.sh:6:"* ]]
}

@test "directory scan reports every finding in the tree" {
  run bash "$LINT" "$FIX"
  [ "$status" -eq 4 ]
  [[ "$output" == *"pos1-count.sh"* ]]
  [[ "$output" == *"pos2-grep-c.sh"* ]]
  [[ "$output" == *"pos3-spaced.sh"* ]]
  [[ "$output" == *"neg6-bare-waiver.sh"* ]]
}

@test "default scope (scripts + .github/workflows) passes on this repo" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
}

@test "unknown option exits 2" {
  run bash "$LINT" --nope
  [ "$status" -eq 2 ]
}

@test "missing path exits 2" {
  run bash "$LINT" "$FIX/does-not-exist.sh"
  [ "$status" -eq 2 ]
}
