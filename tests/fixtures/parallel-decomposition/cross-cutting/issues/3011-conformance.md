## Goal

Add `tests/contracts/conformance.bats` that checks every adapter type against the `ApiResponse` contract.

## Files to read first

- tests/contracts/conformance.bats
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/conformance.bats

## Dependencies

Depends on issue #3001

## Dependency justification

- #3001 — consumes the `ApiResponse` contract added by #3001 (consumes-new-type), which does not exist on the current base branch.

## Files touched

- tests/contracts/conformance.bats

## Acceptance criteria

- [ ] `bats tests/contracts/conformance.bats` exits 0 after the change.
- [ ] `tests/contracts/conformance.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/conformance.bats
```
