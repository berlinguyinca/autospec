## Goal

Add `crates/demo-cycle/src/delta.rs` that derives delta values from gamma values.

## Files to read first

- crates/demo-cycle/src/delta.rs
- crates/demo-cycle/src/gamma.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cycle/delta.bats

## Dependencies

Depends on issue #9003

## Dependency justification

- #9003 — consumes the gamma derivation function added by #9003 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- crates/demo-cycle/src/delta.rs

## Acceptance criteria

- [ ] `bats tests/demo-cycle/delta.bats` exits 0 after the change.
- [ ] `crates/demo-cycle/src/delta.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cycle/delta.bats
```
