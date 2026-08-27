#!/usr/bin/env bats
# tests/unit/test_define_phase0_language.bats — Phase 0 must resolve the
# bootstrap language via classify-language.sh instead of pattern-matching the
# description string.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  export REPO_ROOT
}

@test "Phase 0 calls classify-language.sh on the request" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'classify-language\.sh' "$f"
  done
}

@test "Phase 0 pre-fills the resolved language when unambiguous" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'pre-fill' "$f"
    grep -q 'unambiguous' "$f"
  done
}

@test "Phase 0 asks the operator on a tie" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'ask the operator' "$f"
  done
}

@test "Phase 0 writes the matching .gitignore for the resolved language" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'resolved language' "$f"
    grep -q 'gitignore' "$f"
  done
}

@test "Phase 0 fails closed when classify-language.sh is missing or returns no rank" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'fail closed' "$f"
  done
}
