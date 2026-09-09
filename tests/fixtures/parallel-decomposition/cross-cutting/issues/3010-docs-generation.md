## Goal

Add `docs/generation/src/main.rs` that renders the API reference from the `contracts` crate doc comments.

## Files to read first

- docs/generation/src/main.rs
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/docs.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- docs/generation/src/main.rs

## Acceptance criteria

- [ ] `bats tests/contracts/docs.bats` exits 0 after the change.
- [ ] `docs/generation/src/main.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/docs.bats
```
