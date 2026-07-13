#!/usr/bin/env bats
# tests/autospec-run-agent-env-contract.bats — runtime isolation contract in autospec-run surfaces

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "autospec-run provisions agent-env after worktree assertion" {
  for surface in \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
    grep -q '<!-- agent-env-provision:begin -->' "$surface"
    grep -q 'agent-env.sh" up --repo "$PWD"' "$surface"
    grep -q 'AUTOSPEC_PUBLIC_URL.*canonical browser/QA URL' "$surface"
    grep -q 'agent-env.sh" down --repo /tmp/wt-<BRANCH>' "$surface"
  done
}
