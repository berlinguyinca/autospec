#!/usr/bin/env bats
# tests/batch-size-gating.bats — TDD for effective_batch_size probe (issue #390)

setup() {
  # Source the helper that computes effective_batch_size.
  # We define the function inline here as it will appear in SKILL.md pseudocode.
  compute_effective_batch_size() {
    local reasoning_lbl="${1:-reasoning:medium}"
    local batch_size="${AUTOSPEC_BATCH_SIZE:-1}"
    # Guard against 0 or negative
    [[ "$batch_size" -gt 0 ]] 2>/dev/null || batch_size=1
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

@test "unlabeled issue defaults to reasoning:medium behavior (batch size 1)" {
  result=$(compute_effective_batch_size "reasoning:medium")
  [ "$result" = "1" ]
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

@test "SKILL.md default AUTOSPEC_BATCH_SIZE is 1" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  # Should contain AUTOSPEC_BATCH_SIZE:-1 (the default)
  grep -q 'AUTOSPEC_BATCH_SIZE:-1' "$skill_md"
}

@test "autospec trio default AUTOSPEC_BATCH_SIZE is 1 (parity with autospec-run, issue #971)" {
  for f in SKILL.md codex/prompt.md opencode/agent.md; do
    p="${BATS_TEST_DIRNAME}/../skills/autospec/$f"
    [ -f "$p" ]
    # D2: default must be 1, never the legacy 3.
    grep -q 'AUTOSPEC_BATCH_SIZE:-1' "$p"
    ! grep -qF 'AUTOSPEC_BATCH_SIZE:-3' "$p"
    ! grep -qE 'AUTOSPEC_BATCH_SIZE` issues \(default:? 3\)' "$p"
  done
}

@test "autospec trio carries fresh-subagent-per-issue prose (D2 parity, issue #971)" {
  for f in SKILL.md codex/prompt.md opencode/agent.md; do
    p="${BATS_TEST_DIRNAME}/../skills/autospec/$f"
    grep -qF 'Fresh-subagent-per-issue (canonical Phase 4 path, formerly single-agent absorbed-discipline)' "$p"
    grep -qF 'The orchestrator NEVER implements in its own context' "$p"
    grep -qF 'the default is 1 (one issue per subagent)' "$p"
  done
}

@test "autospec trio posts per-issue token report at pinned slot (D1 parity, issue #971)" {
  for f in SKILL.md codex/prompt.md opencode/agent.md; do
    p="${BATS_TEST_DIRNAME}/../skills/autospec/$f"
    grep -qF '<!-- token-report:begin -->' "$p"
    grep -qF 'post-token-report.sh' "$p"
    # Pinned slot: token-report:begin must fall between SUCCESS step 8 and FAILURE step 9.
    slot=$(awk '
      /(^|> )8\. SUCCESS/ { after=1 }
      /(^|> )9\. FAILURE/ { after=0 }
      after && /token-report:begin/ { print "FOUND" }
    ' "$p")
    [ "$slot" = "FOUND" ]
  done
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

@test "autospec-run documents BATCH_COMPLETE as continuation rather than manual rerun" {
  for f in \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md" \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md" \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
  do
    grep -F 'BATCH_COMPLETE is a continuation signal, not a terminal state' "$f"
    grep -F 'Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE' "$f"
  done
}

@test "reasoning:deep gate is scoped to one monitor batch, not the full autospec-run invocation" {
  for f in \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md" \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md" \
    "${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
  do
    grep -F 'reasoning:deep may reduce a single monitor batch to one issue' "$f"
    grep -F 'the orchestrator MUST relaunch automatically until ALL_DONE' "$f"
  done
}
