#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CODEX_MD="$REPO_ROOT/skills/autospec-run/codex/prompt.md"
  SKILL_MD="$REPO_ROOT/skills/autospec-run/SKILL.md"
  OPENCODE_MD="$REPO_ROOT/skills/autospec-run/opencode/agent.md"
}

@test "autospec-run documents bounded and full-history SpawnAgent shapes" {
  for file in "$CODEX_MD" "$SKILL_MD" "$OPENCODE_MD"; do
    grep -Fq 'Codex SpawnAgent call-shape contract' "$file"
    grep -Fq 'SpawnAgent({ prompt: bounded_handoff, agent_type: "executor", model: TIER_B, reasoning_effort: "medium" })' "$file"
    grep -Fq 'SpawnAgent({ prompt: full_history_prompt, full_history: true })' "$file"
  done
}

@test "autospec-run full-history SpawnAgent shape omits explicit routing fields" {
  for file in "$CODEX_MD" "$SKILL_MD" "$OPENCODE_MD"; do
    grep -Fq 'SpawnAgent({ prompt: full_history_prompt, full_history: true })' "$file"
    run perl -0ne '
      while (/SpawnAgent\(\{.*?\}\)/sg) {
        $call = $&;
        if ($call =~ /full_history/ && $call =~ /(agent_type|model|reasoning_effort)/) {
          print "$ARGV: invalid full-history SpawnAgent shape: $call\n";
          exit 1;
        }
      }
    ' "$file"
    [ "$status" -eq 0 ]
  done
}

@test "autospec-run dispatch failure path retries valid shape or releases claim visibly" {
  for file in "$CODEX_MD" "$SKILL_MD" "$OPENCODE_MD"; do
    grep -Fq 'On Codex dispatch failure, retry once with the other valid shape' "$file"
    grep -Fq 'release the claimed issue back to `auto-implement` with a visible blocker comment' "$file"
  done
}
