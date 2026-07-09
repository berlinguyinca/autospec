# Task 3 Report

## Summary
Implemented the issue intent safety gate wiring for `autospec`, `autospec-define`, and `autospec-classify`. The prompt bodies now include the safety-gate section, the `scripts/lint-issue-safety.sh` callout, `safety:reviewed` / `security:quarantined` label handling, and the `## Safety review` block guidance. The prompt integration test was added first and now passes.

## Verification
- `bats tests/unit/test_phase3_lint_integration.bats`
- `bash scripts/validate.sh --fast`

`validate --fast` gets through the touched skill trios and then fails later in the known unrelated baseline at `tests/phase4/test_docs_drift_gate_regen_conditional.sh` with the existing `skills/autospec-run/SKILL.md` heredoc/baseline issue.

## Changed Files
- `skills/autospec/SKILL.md`
- `skills/autospec/codex/prompt.md`
- `skills/autospec/opencode/agent.md`
- `skills/autospec-define/SKILL.md`
- `skills/autospec-define/codex/prompt.md`
- `skills/autospec-define/opencode/agent.md`
- `skills/autospec-classify/SKILL.md`
- `skills/autospec-classify/codex/prompt.md`
- `skills/autospec-classify/opencode/agent.md`
- `tests/unit/test_phase3_lint_integration.bats`

## Concerns
- The repository still has the pre-existing phase-4 docs-drift baseline failure in `tests/phase4/test_docs_drift_gate_regen_conditional.sh`.

## Review Fix

### RED
- `bats tests/unit/test_phase3_lint_integration.bats` failed on the new order assertion before the prompt rewrite.

### GREEN
- `bats tests/unit/test_phase3_lint_integration.bats` passed after moving the safety gate ahead of the transition / queue-preserving Phase 3.5 steps.
- Targeted lock-step check passed for `skills/autospec`, `skills/autospec-define`, and `skills/autospec-classify` after syncing `SKILL.md`, `codex/prompt.md`, and `opencode/agent.md`.
- `bash scripts/validate.sh --fast` advanced through the touched trios and then failed later on the known unrelated docs-drift baseline in `tests/phase4/test_docs_drift_gate_regen_conditional.sh` (`skills/autospec-run/SKILL.md` heredoc parse failure).

## Review Fix 2

### RED
- `bats tests/unit/test_phase3_lint_integration.bats` failed on the new marker assertion before the prompt regeneration (`grep -q "<!-- autospec-safety:begin -->"`).

### GREEN
- Added marker-delimited `## Safety review` guidance to `skills/autospec-classify/SKILL.md`, `skills/autospec/SKILL.md`, and `skills/autospec-define/SKILL.md`.
- Regenerated `skills/autospec*/codex/prompt.md` and `skills/autospec*/opencode/agent.md` from the updated SKILL sources with the repo trio generator.
- `bats tests/unit/test_phase3_lint_integration.bats` passes.
- `bash scripts/derive-trio.sh skills/autospec --check`
- `bash scripts/derive-trio.sh skills/autospec-define --check`
- `bash scripts/derive-trio.sh skills/autospec-classify --check`
- `bash scripts/validate.sh --fast` now gets through the touched trios and only fails later on the existing phase-4 docs-drift baseline in `tests/phase4/test_docs_drift_gate_regen_conditional.sh` (`skills/autospec-run/SKILL.md` heredoc parse failure).
