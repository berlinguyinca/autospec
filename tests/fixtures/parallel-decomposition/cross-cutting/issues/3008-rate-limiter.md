## Goal

Add `crates/policy/src/rate.rs` that throttles outbound calls with a token-bucket `RateLimiter`.

## Files to read first

- crates/policy/src/rate.rs
- crates/contracts/src/errors.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/policy/rate.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/policy/src/rate.rs

## Acceptance criteria

- [ ] `bats tests/policy/rate.bats` exits 0 after the change.
- [ ] `crates/policy/src/rate.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/policy/rate.bats
```
