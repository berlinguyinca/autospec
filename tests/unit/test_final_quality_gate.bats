#!/usr/bin/env bats
# tests/unit/test_final_quality_gate.bats — Phase 4 final quality gate contract.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  AUTOSPEC_RUN_TRIO=(
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
  )
}

@test "autospec-run trio documents a final quality gate before admin merge" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    grep -q 'Final quality gate' "$f"
    grep -q 'before admin-merging' "$f"
    grep -q 'Do NOT run `gh pr merge` while the final quality gate is failing' "$f"
  done
}

@test "Rust workspaces use clippy all-targets with warnings denied" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    grep -q 'Cargo.toml' "$f"
    grep -q 'cargo clippy --workspace --all-targets -- -D warnings' "$f"
  done
}

@test "final quality gate failure evidence includes crate file line and rule" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    grep -q 'crate' "$f"
    grep -q 'file' "$f"
    grep -q 'line' "$f"
    grep -q 'rule' "$f"
    grep -q 'FINAL_QUALITY_GATE_FAILED' "$f"
  done
}

@test "final quality gate appears before gh pr merge in the success path" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    gate_line="$(grep -n 'Final quality gate' "$f" | head -1 | cut -d: -f1)"
    merge_line="$(grep -n 'gh pr merge <PR> --admin --squash --delete-branch' "$f" | head -1 | cut -d: -f1)"
    [ -n "$gate_line" ]
    [ -n "$merge_line" ]
    [ "$gate_line" -lt "$merge_line" ]
  done
}

@test "repository validate.sh registers the final quality gate suite" {
  grep -q 'tests/unit/test_final_quality_gate.bats' "$REPO_ROOT/scripts/validate.sh"
}
