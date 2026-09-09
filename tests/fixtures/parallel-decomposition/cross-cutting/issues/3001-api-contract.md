## Goal

Add `crates/contracts/src/lib.rs` that declares the shared `ApiResponse` contract for all adapters.

## Files to read first

- crates/contracts/src/lib.rs
- docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/api.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/contracts/src/lib.rs

## Acceptance criteria

- [ ] `bats tests/contracts/api.bats` exits 0 after the change.
- [ ] `crates/contracts/src/lib.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/api.bats
```
