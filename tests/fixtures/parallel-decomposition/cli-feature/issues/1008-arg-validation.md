## Goal

Add `crates/demo-cli/src/validate.rs` that rejects malformed subcommand arguments before dispatch runs.

## Files to read first

- crates/demo-cli/src/validate.rs
- crates/demo-cli/src/dispatch.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/validate.bats

## Dependencies

Depends on issue #1002

## Dependency justification

- #1002 — consumes the `DispatchTable` interface added by #1002 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- crates/demo-cli/src/validate.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/validate.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/validate.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/validate.bats
```
