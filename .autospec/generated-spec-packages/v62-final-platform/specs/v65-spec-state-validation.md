# Spec State And Validation Framework

## Version

V65

## Objective

Model spec lifecycle and run validation gates consistently.

## Scope

- Spec states: planned, ready, running, passed, failed, blocked, deferred, superseded.
- State transition rules.
- Validation command registry.
- Deferred and superseded spec handling.
- JSON state file format.

## Non-Goals

- No agent execution.
- No remote CI integration.

## Dependencies

- `v64-dependency-graph-ordering`

## Files To Create/Modify

- Create: `crates/autospec-core/src/state/mod.rs`
- Create: `crates/autospec-core/src/validation/mod.rs`
- Create: `crates/autospec-core/tests/spec_state.rs`
- Create: `schemas/autospec-spec-state.schema.json`
- Modify: `docs/workflows.md`

## Implementation Steps

1. Define `SpecState` and allowed transitions.
2. Implement state store read/write under `.autospec/state/specs.json`.
3. Implement validation registry entries with command, cwd, timeout, and required flag.
4. Add support for deferred reason and superseded-by spec id.
5. Add tests for valid transitions, invalid transitions, and validation command resolution.

## Acceptance Criteria

- [ ] Invalid state transitions fail.
- [ ] Deferred specs are skipped but reported.
- [ ] Superseded specs point to an existing replacement.
- [ ] Validation registry can run a simple shell command and capture status.

## Validation Commands

```bash
cargo test --all spec_state validation_registry
bash scripts/validate.sh --fast
```

## Expected Outputs

- `.autospec/state/specs.json` can represent package progress.

## Rollback/Handoff Notes

If shell command execution is unsafe in tests, use a fixture command under `tests/fixtures/validation/`.
