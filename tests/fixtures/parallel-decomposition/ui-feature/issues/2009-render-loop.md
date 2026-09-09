## Goal

Add `crates/demo-ui/src/render.rs` that draws the `ViewModel` into the terminal buffer.

## Files to read first

- crates/demo-ui/src/render.rs
- crates/demo-ui/src/view_model.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/render.bats

## Dependencies

Depends on issue #2001

## Dependency justification

- #2001 — consumes the `ViewModel` type added by #2001 (consumes-new-type), which does not exist on the current base branch.

## Files touched

- crates/demo-ui/src/render.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/render.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/render.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/render.bats
```
