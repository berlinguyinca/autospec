# CLI Productization

## Version

V69

## Objective

Expose AutoSpec as a coherent CLI product.

## Scope

Required commands:

- `autospec init`
- `autospec doctor`
- `autospec status`
- `autospec plan`
- `autospec validate`
- `autospec run`
- `autospec resume`
- `autospec report`
- `autospec showcase`
- `autospec benchmark`
- `autospec growth-report`

## Non-Goals

- No hosted UI.
- No replacement of existing skills in this spec.

## Dependencies

- `v68-evidence-release-reporting`

## Files To Create/Modify

- Modify: `crates/autospec-cli/src/main.rs`
- Create: `crates/autospec-cli/src/commands/*.rs`
- Create: `crates/autospec-cli/tests/cli_commands.rs`
- Create: `docs/cli-reference.md`
- Modify: `README.md`

## Implementation Steps

1. Add command modules using `clap`.
2. For each command define purpose, inputs, outputs, JSON mode, error behavior, tests, and docs.
3. Make `doctor`, `status`, `plan`, `validate`, `report`, `showcase`, and `growth-report` support `--json`.
4. Make `run` and `resume` use the execution queue from V66.
5. Add command snapshot tests for help text.
6. Add CLI reference docs.

## Acceptance Criteria

- [ ] Every required command appears in `autospec --help`.
- [ ] JSON mode returns valid JSON for applicable commands.
- [ ] Error behavior is deterministic and documented.
- [ ] CLI reference covers every command.

## Validation Commands

```bash
cargo test --all cli_commands
cargo run --bin autospec -- --help
autospec validate --fast
```

## Expected Outputs

- `autospec doctor --json`
- `autospec status --json`
- `docs/cli-reference.md`

## Rollback/Handoff Notes

If a command cannot be fully implemented, add it as a documented stub that exits non-zero with `not yet implemented` and create a follow-up spec. Do not silently omit commands.
