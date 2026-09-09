## Goal

Add `crates/demo-ui/src/router.rs` that routes key events to handler closures through an `EventRouter`.

## Files to read first

- crates/demo-ui/src/router.rs
- crates/demo-ui/src/view_model.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/router.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/router.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/router.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/router.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/router.bats
```
