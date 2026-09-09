## Goal

Add `tests/policy/integration.bats` that asserts retry and timeout policies compose on one call path.

## Files to read first

- tests/policy/integration.bats
- crates/policy/src/retry.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/policy/integration.bats

## Dependencies

Depends on issue #3006
Depends on issue #3007

## Dependency justification

- #3006 — consumes the `RetryPolicy` interface added by #3006 (consumes-new-interface), which does not exist on the current base branch.
- #3007 — consumes the `TimeoutPolicy` interface added by #3007 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- tests/policy/integration.bats

## Acceptance criteria

- [ ] `bats tests/policy/integration.bats` exits 0 after the change.
- [ ] `tests/policy/integration.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/policy/integration.bats
```
