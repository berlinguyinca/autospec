#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "autospec-run Codex fallback uses bounded non-full-history subagent handoff" {
  local files=(
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
  )

  for file in "${files[@]}"; do
    grep -Fq 'Codex native subagents with explicit `agent_type`, `model`, or `reasoning_effort` MUST use a bounded handoff, not a full-history fork' "$file"
    ! grep -Fq 'for Codex native subagents, fork/inherit the current conversation context' "$file"
  done
}
