#!/usr/bin/env bats
# tests/lint-issue.bats — issue-body quality-gate rule engine coverage
# (issue #3203: vacuous acceptance criteria must be rejected)

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LINTER="$REPO_ROOT/scripts/lint-issue.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/lint-issue"

@test "the new rule id appears in the documentation block" {
  run grep -F 'AC_VACUOUS' "$LINTER"
  [ "$status" -eq 0 ]
  run bash "$LINTER" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"AC_VACUOUS"* ]]
}

@test "an AC asserting 0 occurrences of a literal emits AC_VACUOUS" {
  run bash "$LINTER" "$FIXTURES/vacuous-zero-occurrence.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *AC_VACUOUS* ]]
}

@test "an AC asserting absence of a literal emits AC_VACUOUS" {
  run bash "$LINTER" "$FIXTURES/vacuous-absence.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *AC_VACUOUS* ]]
}

@test "an AC pairing absence with a positive assertion emits 0 findings" {
  run bash "$LINTER" "$FIXTURES/paired-positive.md"
  [ "$status" -eq 0 ]
  [[ -z "$output" ]]
}

@test "both known-vacuous fixtures are flagged" {
  flagged=0
  for fixture in vacuous-zero-occurrence.md vacuous-absence.md; do
    run bash "$LINTER" "$FIXTURES/$fixture"
    if [ "$status" -gt 0 ] && [[ "$output" == *AC_VACUOUS* ]]; then
      flagged=$((flagged + 1))
    fi
  done
  [ "$flagged" -eq 2 ]
}
