## Goal

Add `crates/demo-ui/src/layout.rs` that computes widget cell metrics for the active theme.

## Files to read first

- crates/demo-ui/src/layout.rs
- crates/demo-ui/src/theme.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/layout.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/layout.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/layout.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/layout.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/layout.bats
```
