#!/usr/bin/env bats
# tests/gen-issue-skeleton.bats — bats coverage for gen-issue-skeleton.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
GEN_BIN="$REPO_ROOT/scripts/gen-issue-skeleton.sh"
MINIMAL_YAML="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/minimal.yaml"
VAGUE_YAML="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/vague-goal.yaml"
SECURITY_YAML="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/security-database.yaml"
EXPECTED_MINIMAL="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/expected-minimal.md"
EXPECTED_SECURITY="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/expected-security-database.md"
LINT_BIN="$REPO_ROOT/scripts/lint-issue.sh"

@test "minimal-valid: --input emits issue body with team lenses that passes lint-issue.sh" {
  run bash "$GEN_BIN" --input "$MINIMAL_YAML"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "^## Goal"
  echo "$output" | grep -q "^## Source spec"
  echo "$output" | grep -q "^## Team personality"
  echo "$output" | grep -q "^## Review counter-team"
  echo "$output" | grep -q "^## Files to read first"
  echo "$output" | grep -q "^## Files touched$"
  echo "$output" | grep -q "^## Local-LLM execution notes$"
  echo "$output" | grep -q "^## Dependencies$"
  ! echo "$output" | grep -q "^## Evidence consumed$"
  echo "$output" | grep -q "^## Implementation scope"
  echo "$output" | grep -q "^## Out of scope"
  echo "$output" | grep -q "^## Implementation outline"
  echo "$output" | grep -q "^## Tests required"
  echo "$output" | grep -q "^## Acceptance criteria"
  echo "$output" | grep -q "^## Verification"
  echo "$output" | grep -q "^## Branch name"
  # Output must pass lint-issue.sh
  tmp_body="$(mktemp)"
  echo "$output" > "$tmp_body"
  run bash "$LINT_BIN" "$tmp_body"
  [ "$status" -eq 0 ]

  run diff -u "$EXPECTED_MINIMAL" "$tmp_body"
  rm -f "$tmp_body"
  [ "$status" -eq 0 ]
}

@test "security profile renders validated security context and matches its golden" {
  run bash "$GEN_BIN" --input "$SECURITY_YAML"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "^## Evidence consumed$"
  echo "$output" | grep -q "^## Controls covered$"
  echo "$output" | grep -q "^## Prerequisites$"

  tmp_body="$(mktemp)"
  printf '%s\n' "$output" > "$tmp_body"
  run bash "$LINT_BIN" "$tmp_body"
  [ "$status" -eq 0 ]
  run diff -u "$EXPECTED_SECURITY" "$tmp_body"
  rm -f "$tmp_body"
  [ "$status" -eq 0 ]
}

@test "common profile fields fail closed when absent" {
  tmp_yaml="$(mktemp).yaml"
  grep -v '^files_touched:' "$MINIMAL_YAML" | sed '/^  - scripts\/gen-issue-skeleton.sh$/d; /^  - tests\/gen-issue-skeleton.bats$/d' > "$tmp_yaml"
  run bash "$GEN_BIN" --input "$tmp_yaml"
  rm -f "$tmp_yaml"
  [ "$status" -ne 0 ]
  [[ "$output" == *"MISSING_FIELD:files_touched"* ]]
}

@test "security profile fields fail closed when absent" {
  tmp_yaml="$(mktemp).yaml"
  sed '/^controls_covered:/,/^[a-z_]*:/ { /^controls_covered:/d; /^  - /d; }' "$SECURITY_YAML" > "$tmp_yaml"
  run bash "$GEN_BIN" --input "$tmp_yaml"
  rm -f "$tmp_yaml"
  [ "$status" -ne 0 ]
  [[ "$output" == *"MISSING_FIELD:controls_covered"* ]]
}

@test "missing-required-field: exits non-zero with MISSING_FIELD on stderr" {
  # Create a YAML missing the required goal_sentence field
  tmp_yaml="$(mktemp).yaml"
  cat > "$tmp_yaml" <<'YAML'
issue_id: missing-goal
spec_path: docs/specs/foo.md
spec_url: https://example.com
team_personality:
  - "Core product engineering: product manager, architect, backend developer, test engineer"
review_counter_team:
  - "Security and reliability review: security advisor, SRE, regression tester"
files_to_read:
  - scripts/lint-issue.sh
implementation_scope:
  - "scripts/example.sh"
out_of_scope:
  - "Nothing"
implementation_outline_lines:
  - "Do the thing"
tests_required:
  - "bats tests/example.bats"
acceptance_criteria:
  - "scripts/example.sh exits 0"
verification:
  primary_smoke: "bats tests/example.bats"
  operator_full: "bash scripts/example.sh"
branch_name: feat/example
YAML
  run bash "$GEN_BIN" --input "$tmp_yaml"
  rm -f "$tmp_yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "MISSING_FIELD:goal_sentence" ]] || [[ "${lines[@]}" =~ "MISSING_FIELD:goal_sentence" ]]
}

@test "lint-rejects-vague-goal: vague goal_sentence causes non-zero exit" {
  run bash "$GEN_BIN" --input "$VAGUE_YAML"
  # Should fail because lint-issue.sh will reject the vague goal
  [ "$status" -ne 0 ]
}

@test "operator-full-pipe: generated body can be linted via /dev/stdin" {
  run bash -c "bash '$GEN_BIN' --input '$MINIMAL_YAML' | bash '$LINT_BIN' /dev/stdin"
  [ "$status" -eq 0 ]
}
