#!/usr/bin/env bats
# tests/unit/test_autospec_sweep_skill.bats — skill and autospec integration contract.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "autospec-sweep ships as a lock-step multi-harness skill" {
  for f in \
    "$REPO_ROOT/skills/autospec-sweep/SKILL.md" \
    "$REPO_ROOT/skills/autospec-sweep/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-sweep/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-sweep/README.md" \
    "$REPO_ROOT/skills/autospec-sweep/install.sh" \
    "$REPO_ROOT/skills/autospec-sweep/uninstall.sh"
  do
    [ -f "$f" ]
  done
}

@test "autospec first-run preflight routes missing config through autospec-sweep" {
  for f in \
    "$REPO_ROOT/skills/autospec/SKILL.md" \
    "$REPO_ROOT/skills/autospec/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec/codex/prompt.md"
  do
    grep -q '.autospec/autospec.yml' "$f"
    grep -q '/autospec-sweep init' "$f"
  done
}

@test "validate.sh includes autospec-sweep in generated-skill invariants" {
  grep -q 'autospec-sweep' "$REPO_ROOT/autospec validate"
  grep -q 'check_autospec_sweep_config_contract' "$REPO_ROOT/autospec validate"
}
