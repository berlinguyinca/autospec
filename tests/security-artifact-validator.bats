#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  VALIDATOR="$REPO_ROOT/scripts/validate-security-artifact.py"
  FIXTURES="$REPO_ROOT/tests/fixtures/security-artifact"
}

assert_rejected_with() {
  local fixture="$1"
  local rule_id="$2"

  run python3 "$VALIDATOR" "$FIXTURES/$fixture"
  [ "$status" -ne 0 ]
  [[ "$output" == *"$rule_id:"* ]]
}

@test "accepts a complete security artifact" {
  run python3 "$VALIDATOR" "$FIXTURES/valid.yml"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "requires authoritative ownership for catastrophic controls" {
  assert_rejected_with "missing-authority.yml" "AUTHORITATIVE_CONTROL_MISSING"
}

@test "requires every control to own a negative test" {
  assert_rejected_with "control-without-test.yml" "CONTROL_WITHOUT_TEST"
}

@test "rejects unresolved evidence consumed by an issue" {
  assert_rejected_with "unresolved-evidence.yml" "EVIDENCE_UNRESOLVED"
}

@test "keeps issues with blocking prerequisites out of the queue" {
  assert_rejected_with "blocked-queued.yml" "BLOCKING_PREREQUISITE_QUEUED"
}

@test "rejects unknown issue dependencies" {
  assert_rejected_with "unknown-dependency.yml" "DEPENDENCY_UNKNOWN"
}

@test "rejects cyclic issue dependencies" {
  assert_rejected_with "cyclic-dependency.yml" "DEPENDENCY_CYCLE"
}

@test "requires every spec section and negative test to have an owner" {
  run python3 "$VALIDATOR" "$FIXTURES/uncovered.yml"
  [ "$status" -ne 0 ]
  [[ "$output" == *"SPEC_SECTION_UNCOVERED:"* ]]
  [[ "$output" == *"NEGATIVE_TEST_UNOWNED:"* ]]
}

@test "keeps atomic contracts in one issue" {
  assert_rejected_with "atomic-split.yml" "ATOMIC_CONTRACT_SPLIT"
}

@test "reports malformed YAML without a traceback" {
  run python3 "$VALIDATOR" "$FIXTURES/malformed.yml"
  [ "$status" -ne 0 ]
  [[ "$output" == *"PROFILE_SCHEMA_INVALID:"* ]]
  [[ "$output" != *"Traceback"* ]]
}

@test "emits stable JSON findings" {
  run python3 "$VALIDATOR" --json "$FIXTURES/missing-authority.yml"
  [ "$status" -ne 0 ]
  run python3 -c 'import json,sys; data=json.loads(sys.argv[1]); assert data[0]["rule_id"] == "AUTHORITATIVE_CONTROL_MISSING"' "$output"
  [ "$status" -eq 0 ]
}

@test "reports a missing YAML parser without a traceback" {
  run python3 -S "$VALIDATOR" "$FIXTURES/valid.yml"
  [ "$status" -ne 0 ]
  [[ "$output" == *"PROFILE_SCHEMA_INVALID:"* ]]
  [[ "$output" != *"Traceback"* ]]
}

@test "prints usage" {
  run python3 "$VALIDATOR" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"validate-security-artifact.py"* ]]
}
