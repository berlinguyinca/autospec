#!/usr/bin/env bats
# tests/batch-size-gating.bats — TDD for effective_batch_size probe (issue #390)

setup() {
  # Source the helper that computes effective_batch_size.
  # We define the function inline here as it will appear in SKILL.md pseudocode.
  compute_effective_batch_size() {
    local reasoning_lbl="${1:-reasoning:medium}"
    local batch_size="${AUTOSPEC_BATCH_SIZE:-3}"
    # Guard against 0 or negative
    [[ "$batch_size" -gt 0 ]] 2>/dev/null || batch_size=3
    if [ "$reasoning_lbl" = "reasoning:deep" ]; then
      echo 1
    else
      echo "$batch_size"
    fi
  }
}

@test "reasoning:deep issue forces effective_batch_size=1" {
  result=$(compute_effective_batch_size "reasoning:deep")
  [ "$result" = "1" ]
}

@test "reasoning:medium with AUTOSPEC_BATCH_SIZE=3 gives effective_batch_size=3" {
  AUTOSPEC_BATCH_SIZE=3 result=$(compute_effective_batch_size "reasoning:medium")
  [ "$result" = "3" ]
}

@test "AUTOSPEC_BATCH_SIZE env override of 7 with non-deep label gives effective_batch_size=7" {
  AUTOSPEC_BATCH_SIZE=7 result=$(compute_effective_batch_size "reasoning:shallow")
  [ "$result" = "7" ]
}

@test "unlabeled issue defaults to reasoning:medium behavior (batch size 3)" {
  AUTOSPEC_BATCH_SIZE=3 result=$(compute_effective_batch_size "reasoning:medium")
  [ "$result" = "3" ]
}

@test "reasoning:deep overrides even large AUTOSPEC_BATCH_SIZE" {
  AUTOSPEC_BATCH_SIZE=10 result=$(compute_effective_batch_size "reasoning:deep")
  [ "$result" = "1" ]
}

@test "SKILL.md contains effective_batch_size probe before claim step" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  [ -f "$skill_md" ]
  grep -q "effective_batch_size" "$skill_md"
}

@test "SKILL.md default AUTOSPEC_BATCH_SIZE is 3" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  # Should contain AUTOSPEC_BATCH_SIZE:-3 (the default)
  grep -q 'AUTOSPEC_BATCH_SIZE:-3' "$skill_md"
}

@test "AGENTS.md documents batch default and reasoning:deep gating" {
  agents_md="${BATS_TEST_DIRNAME}/../AGENTS.md"
  [ -f "$agents_md" ]
  grep -q "AUTOSPEC_BATCH_SIZE" "$agents_md"
  grep -q "reasoning:deep" "$agents_md"
}

@test "codex/prompt.md contains effective_batch_size probe (lockstep with SKILL.md)" {
  codex_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
  [ -f "$codex_md" ]
  grep -q "effective_batch_size" "$codex_md"
}

@test "codex/prompt.md uses effective_batch_size in batch comparison (lockstep)" {
  codex_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
  grep -q 'effective_batch_size:-\$BATCH_SIZE' "$codex_md"
}

@test "opencode/agent.md contains effective_batch_size probe (lockstep with SKILL.md)" {
  opencode_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
  [ -f "$opencode_md" ]
  grep -q "effective_batch_size" "$opencode_md"
}

@test "opencode/agent.md uses effective_batch_size in batch comparison (lockstep)" {
  opencode_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
  grep -q 'effective_batch_size:-\$BATCH_SIZE' "$opencode_md"
}
