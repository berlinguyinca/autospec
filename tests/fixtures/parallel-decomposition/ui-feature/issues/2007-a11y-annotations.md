## Goal

Add `crates/demo-ui/src/a11y.rs` that attaches accessibility annotations to rendered TUI widgets.

## Files to read first

- crates/demo-ui/src/a11y.rs
- crates/demo-ui/src/view_model.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/a11y.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-ui/src/a11y.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/a11y.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/a11y.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/a11y.bats
```
