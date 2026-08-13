#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SKILL="$REPO_ROOT/skills/autospec-run/SKILL.md"
}

@test "ready selection excludes blocked prerequisite issues" {
  grep -Fq 'autospec:blocked-prerequisite' "$SKILL"
  grep -Fq 'all_open excludes every issue carrying' "$SKILL"
}

@test "predispatch gate accepts only absent or verified prerequisites" {
  grep -Fq 'Security prerequisite pre-dispatch gate' "$SKILL"
  grep -Fq 'verified:' "$SKILL"
  grep -Fq 'code_health:security_prerequisite_blocked' "$SKILL"
  grep -Fq -- '--remove-label auto-implement --add-label autospec:blocked-prerequisite' "$SKILL"
}

@test "autospec-run prerequisite contract is lock-step" {
  "$REPO_ROOT/scripts/derive-trio.sh" "$REPO_ROOT/skills/autospec-run" --check
  for file in \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
  do
    grep -Fq 'code_health:security_prerequisite_blocked' "$file"
  done
}
