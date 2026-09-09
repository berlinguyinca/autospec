## Goal

Add `crates/policy/src/timeout.rs` that caps blocking calls with a configurable `TimeoutPolicy`.

## Files to read first

- crates/policy/src/timeout.rs
- crates/contracts/src/errors.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/policy/timeout.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/policy/src/timeout.rs

## Acceptance criteria

- [ ] `bats tests/policy/timeout.bats` exits 0 after the change.
- [ ] `crates/policy/src/timeout.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/policy/timeout.bats
```
