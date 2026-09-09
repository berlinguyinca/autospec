## Goal

Add `crates/flags/src/lib.rs` that reads feature flags from environment variables at startup.

## Files to read first

- crates/flags/src/lib.rs
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/flags/flags.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/flags/src/lib.rs

## Acceptance criteria

- [ ] `bats tests/flags/flags.bats` exits 0 after the change.
- [ ] `crates/flags/src/lib.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/flags/flags.bats
```
