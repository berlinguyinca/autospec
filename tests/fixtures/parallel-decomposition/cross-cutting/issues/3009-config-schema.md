## Goal

Add `crates/contracts/src/config.rs` that validates the shared deployment config schema.

## Files to read first

- crates/contracts/src/config.rs
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/config.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/contracts/src/config.rs

## Acceptance criteria

- [ ] `bats tests/contracts/config.bats` exits 0 after the change.
- [ ] `crates/contracts/src/config.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/config.bats
```
