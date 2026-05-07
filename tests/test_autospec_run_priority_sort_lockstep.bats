#!/usr/bin/env bats
@test "priority sort block exists in SKILL.md" {
  grep -q "Queue priority sort" skills/autospec-run/SKILL.md
}
@test "priority sort block byte-identical: SKILL.md vs opencode" {
  diff <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/opencode/agent.md | head -20)
}
@test "priority sort block byte-identical: SKILL.md vs codex" {
  diff <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/codex/prompt.md | head -20)
}
