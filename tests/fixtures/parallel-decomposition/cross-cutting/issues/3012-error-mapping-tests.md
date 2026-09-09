## Goal

Add `tests/contracts/errors.bats` that asserts each `ContractError` variant maps to one exit code.

## Files to read first

- tests/contracts/errors.bats
- crates/contracts/src/errors.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/errors.bats

## Dependencies

Depends on issue #3002

## Dependency justification

- #3002 — consumes the `ContractError` taxonomy added by #3002 (consumes-new-type), which does not exist on the current base branch.

## Files touched

- tests/contracts/errors.bats

## Acceptance criteria

- [ ] `bats tests/contracts/errors.bats` exits 0 after the change.
- [ ] `tests/contracts/errors.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/errors.bats
```
