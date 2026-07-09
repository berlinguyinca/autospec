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
