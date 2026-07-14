#!/usr/bin/env bats
# tests/unit/test_phase1_bounded_context.bats — Phase 1 research must not
# inherit long parent conversations into a subagent compact task.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  export REPO_ROOT
}

@test "autospec Phase 1 requires a bounded fresh-context research handoff" {
  for f in \
    "$REPO_ROOT/skills/autospec/SKILL.md" \
    "$REPO_ROOT/skills/autospec/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
  do
    grep -q 'Phase 1 bounded-context rule' "$f"
    grep -q 'Do NOT fork, inherit, or compact the full parent conversation' "$f"
    grep -q 'fork_context=false' "$f"
    grep -q 'context window or remote compact failure' "$f"
    grep -q 'bounded local read-only `rg`/file-read investigation' "$f"
  done
}

@test "validate.sh enforces the Phase 1 bounded context contract" {
  grep -q '^check_phase1_bounded_context_contract()' "$REPO_ROOT/autospec validate"
  count="$(grep -c 'check_phase1_bounded_context_contract' "$REPO_ROOT/autospec validate")"
  [ "$count" -ge 2 ]
}
