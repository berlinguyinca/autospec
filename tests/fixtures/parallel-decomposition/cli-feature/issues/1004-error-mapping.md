## Goal

Add `crates/demo-cli/src/errors.rs` that maps internal failures to exit codes and one-line stderr messages.

## Files to read first

- crates/demo-cli/src/errors.rs
- crates/demo-cli/src/dispatch.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/errors.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-cli/src/errors.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/errors.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/errors.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/errors.bats
```
