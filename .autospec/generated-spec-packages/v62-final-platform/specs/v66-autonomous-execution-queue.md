# Autonomous Execution Queue

## Version

V66

## Objective

Add a resumable execution queue for ordered specs.

## Scope

- Queue creation from ordered specs.
- Run state persistence.
- Resume after interruption.
- Failure classification.
- Retry policy.
- Handoff files and rollback notes.
- Human checkpoint gates.
- Final run report.

## Non-Goals

- No specific agent integration beyond placeholder runner trait.
- No destructive operations without safe-mode gate.

## Dependencies

- `v65-spec-state-validation`

## Files To Create/Modify

- Create: `crates/autospec-core/src/execution/mod.rs`
- Create: `crates/autospec-core/src/execution/queue.rs`
- Create: `crates/autospec-core/src/execution/report.rs`
- Create: `crates/autospec-core/tests/execution_queue.rs`
- Create: `schemas/autospec-run-report.schema.json`
- Modify: `docs/workflows.md`

## Implementation Steps

1. Implement queue entries with spec id, status, attempts, timestamps, and validation result.
2. Persist queue state under `.autospec/runs/<run-id>/queue.json`.
3. Implement resume by loading the latest incomplete run.
4. Classify failures as validation, environment, agent, dependency, or safety.
5. Generate handoff markdown on blocked state.
6. Generate final run report in markdown and JSON.

## Acceptance Criteria

- [ ] Queue resumes from interrupted state.
- [ ] Retry limit is enforced.
- [ ] Blocked specs generate handoff files.
- [ ] Final report includes passed, failed, blocked, deferred, and superseded specs.

## Validation Commands

```bash
cargo test --all execution_queue
bash scripts/validate.sh --fast
```

## Expected Outputs

- `.autospec/runs/<run-id>/report.md`
- `.autospec/runs/<run-id>/report.json`

## Rollback/Handoff Notes

If existing `.autospec/run-summary.md` conflicts with the new format, preserve old files and add a converter spec later.
