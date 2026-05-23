#!/usr/bin/env bats
# tests/gen-issue-skeleton.bats — bats coverage for gen-issue-skeleton.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
GEN_BIN="$REPO_ROOT/scripts/gen-issue-skeleton.sh"
MINIMAL_YAML="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/minimal.yaml"
VAGUE_YAML="$REPO_ROOT/tests/fixtures/gen-issue-skeleton/vague-goal.yaml"
LINT_BIN="$REPO_ROOT/scripts/lint-issue.sh"

@test "minimal-valid: --input emits 11-section body that passes lint-issue.sh" {
  run bash "$GEN_BIN" --input "$MINIMAL_YAML"
  [ "$status" -eq 0 ]
  # Must contain all 11 required section headings
  echo "$output" | grep -q "^## Goal"
  echo "$output" | grep -q "^## Source spec"
  echo "$output" | grep -q "^## Files to read first"
  echo "$output" | grep -q "^## Local-LLM notes"
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
  rm -f "$tmp_body"
  [ "$status" -eq 0 ]
}

@test "missing-required-field: exits non-zero with MISSING_FIELD on stderr" {
  # Create a YAML missing the required goal_sentence field
  tmp_yaml="$(mktemp).yaml"
  cat > "$tmp_yaml" <<'YAML'
issue_id: missing-goal
spec_path: docs/specs/foo.md
spec_url: https://example.com
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
