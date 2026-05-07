#!/usr/bin/env bats
# Verify autospec-review skill body is byte-identical across SKILL.md,
# opencode/agent.md, and codex/prompt.md.

@test "skill body byte-identity: SKILL.md vs opencode/agent.md" {
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md)
}

@test "skill body byte-identity: SKILL.md vs codex/prompt.md" {
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(cat skills/autospec-review/codex/prompt.md)
}
