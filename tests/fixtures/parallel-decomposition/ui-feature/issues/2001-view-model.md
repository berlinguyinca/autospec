## Goal

Add `crates/demo-ui/src/view_model.rs` that builds the `ViewModel` consumed by the render loop.

## Files to read first

- crates/demo-ui/src/view_model.rs
- crates/demo-ui/src/main.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/view-model.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/view_model.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/view-model.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/view_model.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/view-model.bats
```
