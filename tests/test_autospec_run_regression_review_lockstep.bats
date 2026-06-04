#!/usr/bin/env bats
# Tier right-sizing (issue #941, spec §D4): the second Tier-A regression
# meta-review dispatch is folded into the single fused reviewer brief, the
# reviewer runs Tier B for ALL issues, and AUTOSPEC_REVIEWER_TIER=opus is the
# escape hatch back to Tier A. The folded gap-check + reviewer-lessons
# write-path must stay byte-identical across the run trio.

RUN_TRIO=(
  skills/autospec-run/SKILL.md
  skills/autospec-run/opencode/agent.md
  skills/autospec-run/codex/prompt.md
)

@test "no second Tier-A regression meta-review dispatch remains in run trio" {
  for f in "${RUN_TRIO[@]}"; do
    ! grep -q 'dispatch a second `TIER_A` subagent' "$f"
  done
}

@test "regression gap-check folded into single reviewer brief (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -qi "would the reviewer have caught the original gap" "$f"
    grep -q "reviewer-lessons.md" "$f"
  done
}

@test "reviewer Model tier is TIER_B for ALL issues (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q '`TIER_B` for ALL issues' "$f"
  done
}

@test "AUTOSPEC_REVIEWER_TIER env hatch documented in run trio" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q "AUTOSPEC_REVIEWER_TIER" "$f"
  done
}

@test "folded reviewer item 9 byte-identical: SKILL.md vs opencode" {
  diff <(grep -n "Regression gap-check" skills/autospec-run/SKILL.md | sed 's/^[0-9]*://') \
       <(grep -n "Regression gap-check" skills/autospec-run/opencode/agent.md | sed 's/^[0-9]*://')
}

@test "folded reviewer item 9 byte-identical: SKILL.md vs codex" {
  diff <(grep -n "Regression gap-check" skills/autospec-run/SKILL.md | sed 's/^[0-9]*://') \
       <(grep -n "Regression gap-check" skills/autospec-run/codex/prompt.md | sed 's/^[0-9]*://')
}
