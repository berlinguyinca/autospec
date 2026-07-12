#!/usr/bin/env bats
# Smoke regression for issue #1841: the lock-step autospec-run prompt tells
# implementers to reset repo-local state roots after entering the worktree.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "autospec-run prompt contains worktree repo-state reset" {
  grep -q 'export AUTOSPEC_REPO_DIR="$PWD"' "$REPO_ROOT/skills/autospec-run/SKILL.md"
  grep -q 'premerge/validation helpers must read mutable artifacts' "$REPO_ROOT/skills/autospec-run/SKILL.md"
}
