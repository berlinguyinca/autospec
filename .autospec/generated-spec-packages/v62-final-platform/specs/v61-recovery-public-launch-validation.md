# V61 Recovery: Public Launch Validation

## Version

V61

## Objective

Turn V61 from launch-readiness artifacts into verified public launch readiness.

## Scope

- Re-run full validation in a git-worktree-capable environment.
- Resolve or supersede stale `.autospec/qa-verdict.json`.
- Flip final launch candidate state only after proof.
- Preserve existing V61 docs, demo, templates, and marketing assets.

## Non-Goals

- No new autonomy features.
- No Rust workspace work.

## Dependencies

None.

## Files To Create/Modify

- Modify: `.autospec/releases/launch-candidate.md`
- Modify: `.autospec/reports/final-launch-readiness.md`
- Modify: `.autospec/handoff/codex-final-handoff.md`
- Modify or supersede: `.autospec/qa-verdict.json`
- Optional delete after proof: `.autospec/handoff/validation-blocker.md`

## Implementation Steps

1. Run `autospec validate` in an environment that can create git worktrees.
2. If `tests/autospec-run/test_parallel_dispatch.bats` still fails, fix `scripts/dispatch-implementer.sh`, `scripts/worktree-guard.sh`, or the test fixture.
3. Regenerate or supersede stale QA verdict evidence so current HEAD is represented.
4. Change launch candidate and final report gate from false to true only after full validation passes.
5. Run `bash scripts/validate-public-launch-readiness.sh`.

## Acceptance Criteria

- [ ] `autospec validate` exits 0.
- [ ] `bash scripts/validate-public-launch-readiness.sh` prints `AUTOSPEC_PUBLIC_LAUNCH_READY=true`.
- [ ] Stale QA verdict is not used as current launch evidence.
- [ ] Handoff file documents any remaining non-blocking warnings.

## Validation Commands

```bash
autospec validate
bash scripts/validate-public-launch-readiness.sh
```

## Expected Outputs

- `AUTOSPEC_PUBLIC_LAUNCH_READY=true`
- Updated launch candidate report.

## Rollback/Handoff Notes

If worktree tests fail in a normal environment, keep public launch false and write `.autospec/handoff/v61-recovery-public-launch-validation-blocker.md`.
