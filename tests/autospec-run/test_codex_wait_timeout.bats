#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  FILES=(
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
  )
}

@test "autospec-run enforces the native Codex wait timeout floor in every harness" {
  for file in "${FILES[@]}"; do
    grep -Fq 'Codex Wait call-shape contract' "$file"
    grep -Fq 'native Codex `wait_agent`' "$file"
    grep -Fq 'omit `timeout_ms` or pass an integer greater than or equal to `10000`' "$file"
    grep -Fq 'Never pass `timeout_ms` below `10000`' "$file"
  done
}

@test "native Codex wait timeout contract guards the monitor completion wait" {
  for file in "${FILES[@]}"; do
    contract_line="$(grep -n -m1 'Codex Wait call-shape contract' "$file" | cut -d: -f1)"
    loop_line="$(grep -n -m1 'wait for task-notification (monitor agent completes)' "$file" | cut -d: -f1)"

    [ -n "$contract_line" ]
    [ -n "$loop_line" ]
    [ "$contract_line" -lt "$loop_line" ]
    [ $((loop_line - contract_line)) -le 12 ]
  done
}
