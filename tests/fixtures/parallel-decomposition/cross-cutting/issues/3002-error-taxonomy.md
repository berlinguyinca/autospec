## Goal

Add `crates/contracts/src/errors.rs` that defines the `ContractError` taxonomy shared by every adapter.

## Files to read first

- crates/contracts/src/errors.rs
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/taxonomy.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/contracts/src/errors.rs

## Acceptance criteria

- [ ] `bats tests/contracts/taxonomy.bats` exits 0 after the change.
- [ ] `crates/contracts/src/errors.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/taxonomy.bats
```
