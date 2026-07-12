#!/usr/bin/env bats
# Regression for issue #1841: Phase 4 issue worktrees must reset
# AUTOSPEC_REPO_DIR so premerge/validation helpers read mutable state from the
# active worktree, not the long-lived parent checkout.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
TRIO=(
  "skills/autospec-run/SKILL.md"
  "skills/autospec-run/codex/prompt.md"
  "skills/autospec-run/opencode/agent.md"
)

@test "autospec-run trio resets AUTOSPEC_REPO_DIR after worktree ladder cd" {
  for rel in "${TRIO[@]}"; do
    file="$REPO_ROOT/$rel"
    grep -q 'export AUTOSPEC_REPO_DIR="$PWD"' "$file"
    grep -q 'premerge/validation helpers must read mutable artifacts' "$file"
  done
}

@test "autospec-run mirrors stay derived after env reset edit" {
  run bash "$REPO_ROOT/scripts/derive-trio.sh" "$REPO_ROOT/skills/autospec-run" --check
  [ "$status" -eq 0 ]
}
