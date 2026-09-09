## Goal

Add `crates/demo-ui/src/session.rs` that persists `ScreenStore` state on exit through the `EventRouter`.

## Files to read first

- crates/demo-ui/src/session.rs
- crates/demo-ui/src/router.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/session.bats

## Dependencies

Depends on issue #2002
Depends on issue #2003

## Dependency justification

- #2002 — consumes the `EventRouter` interface added by #2002 (consumes-new-interface), which does not exist on the current base branch.
- #2003 — consumes the `ScreenStore` type added by #2003 (consumes-new-type), which does not exist on the current base branch.

## Files touched

- crates/demo-ui/src/session.rs

## Acceptance criteria

- [ ] `bats tests/demo-ui/session.bats` exits 0 after the change.
- [ ] `crates/demo-ui/src/session.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/session.bats
```
