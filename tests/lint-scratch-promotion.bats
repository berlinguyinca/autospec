#!/usr/bin/env bats
# tests/lint-scratch-promotion.bats
#
# Exercises scripts/lint-scratch-promotion.sh (issue #3977) against a
# populated positive case (#3793) and the boundary / negative cases:
# a tool invoked from a scratch path more than twice is reported for
# promotion; a tool used twice or fewer, a repo-path tool, an ephemeral
# mktemp helper, a data-only scratch path, and a commented invocation are not.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-scratch-promotion.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-scratch-promotion"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"SCRATCH_PROMOTION"* ]]
}

@test "populated #3793 case: a scratch tool invoked three times is flagged" {
  run bash "$LINT" "$FIX/pos1-populated-3793.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SCRATCH_PROMOTION:"* ]]
  [[ "$output" == *"/tmp/gw-as-3977-22910004/tools/convert.sh"* ]]
  [[ "$output" == *"invoked 3x"* ]]
  [[ "$output" == *"pos1-populated-3793.sh"* ]]
}

@test "populated case does NOT flag the tool used exactly twice (boundary)" {
  run bash "$LINT" "$FIX/pos1-populated-3793.sh"
  [ "$status" -eq 1 ]
  # the 3x convert.sh is flagged; the 2x verify.sh is not
  [[ "$output" != *"verify.sh"* ]]
}

@test "populated case does NOT count data-only scratch paths as invocations" {
  run bash "$LINT" "$FIX/pos1-populated-3793.sh"
  [ "$status" -eq 1 ]
  # convert.sh is invoked 3x (not 4x) even though it is also rm'd as data
  [[ "$output" == *"invoked 3x"* ]]
  [[ "$output" != *"invoked 4x"* ]]
}

@test "scratch tool invoked exactly twice is not a finding (boundary)" {
  run bash "$LINT" "$FIX/neg1-boundary-two.sh"
  [ "$status" -eq 0 ]
}

@test "a repo-path tool is never a scratch-promotion finding" {
  run bash "$LINT" "$FIX/neg2-repo-path.sh"
  [ "$status" -eq 0 ]
}

@test "an ephemeral mktemp-style helper is exempt" {
  run bash "$LINT" "$FIX/neg3-ephemeral.sh"
  [ "$status" -eq 0 ]
}

@test "scratch paths used only as data are not invocations" {
  run bash "$LINT" "$FIX/neg4-data-only.sh"
  [ "$status" -eq 0 ]
}

@test "commented-out invocations are not counted" {
  run bash "$LINT" "$FIX/neg5-commented.sh"
  [ "$status" -eq 0 ]
}

@test "directory scan counts the same tool across agents (2 + 1 = 3)" {
  run bash "$LINT" "$FIX/dir"
  [ "$status" -eq 1 ]
  [[ "$output" == *"SCRATCH_PROMOTION:"* ]]
  [[ "$output" == *"/tmp/gw-as-3977-22910009/tools/split.sh"* ]]
  [[ "$output" == *"invoked 3x"* ]]
}

@test "whole-fixtures scan reports each promotion candidate once" {
  run bash "$LINT" "$FIX"
  [ "$status" -eq 2 ]
  [ "$(printf '%s\n' "$output" | grep -c '^SCRATCH_PROMOTION:')" -eq 2 ]
}

@test "--list emits an audit line per scratch tool and exits 0" {
  run bash "$LINT" --list "$FIX/pos1-populated-3793.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"SCRATCH_TOOL:/tmp/gw-as-3977-22910004/tools/convert.sh: 3x"* ]]
  [[ "$output" == *"SCRATCH_TOOL:/tmp/gw-as-3977-22910004/tools/verify.sh: 2x"* ]]
  # the ephemeral helper is exempt from the audit too
  [[ "$output" != *"XXXXXX"* ]]
}

@test "default scan of scripts/ and skills/ passes on this tree" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
}
