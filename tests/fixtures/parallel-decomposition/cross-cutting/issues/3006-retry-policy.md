## Goal

Add `crates/policy/src/retry.rs` that retries idempotent calls with exponential backoff in a `RetryPolicy`.

## Files to read first

- crates/policy/src/retry.rs
- crates/contracts/src/errors.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/policy/retry.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/policy/src/retry.rs

## Acceptance criteria

- [ ] `bats tests/policy/retry.bats` exits 0 after the change.
- [ ] `crates/policy/src/retry.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/policy/retry.bats
```
