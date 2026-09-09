## Goal

Add `crates/demo-ui/src/store.rs` that keeps screen state in a `ScreenStore` with typed getters.

## Files to read first

- crates/demo-ui/src/store.rs
- crates/demo-ui/src/view_model.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/store.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/store.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/store.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/store.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/store.bats
```
