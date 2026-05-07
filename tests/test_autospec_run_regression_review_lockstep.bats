#!/usr/bin/env bats
@test "regression review escalation block exists in SKILL.md" {
  grep -q "Regression review escalation" skills/autospec-run/SKILL.md
}
@test "regression review escalation block byte-identical: SKILL.md vs opencode" {
  diff <(sed -n '/Regression review escalation/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Regression review escalation/,/^---/p' skills/autospec-run/opencode/agent.md | head -20)
}
@test "regression review escalation block byte-identical: SKILL.md vs codex" {
  diff <(sed -n '/Regression review escalation/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Regression review escalation/,/^---/p' skills/autospec-run/codex/prompt.md | head -20)
}
