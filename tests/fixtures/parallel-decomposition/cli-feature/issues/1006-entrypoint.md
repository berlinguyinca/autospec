## Goal

Add `crates/demo-cli/src/main.rs` that wires argument parsing to the `DispatchTable` and error handler.

## Files to read first

- crates/demo-cli/src/main.rs
- crates/demo-cli/src/config.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/main.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-cli/src/main.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/main.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/main.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/main.bats
```
