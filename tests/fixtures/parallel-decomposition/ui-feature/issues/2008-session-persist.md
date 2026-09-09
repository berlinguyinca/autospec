## Goal

Add `crates/demo-ui/src/persist.rs` that encodes and decodes `ScreenStore` sessions to JSON.

## Files to read first

- crates/demo-ui/src/persist.rs
- crates/demo-ui/src/store.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/persist.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/persist.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/persist.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/persist.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/persist.bats
```
