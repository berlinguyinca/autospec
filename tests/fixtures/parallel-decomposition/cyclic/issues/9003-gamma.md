## Goal

Add `crates/demo-cycle/src/gamma.rs` that derives gamma values from beta values.

## Files to read first

- crates/demo-cycle/src/gamma.rs
- crates/demo-cycle/src/beta.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cycle/gamma.bats

## Dependencies

Depends on issue #9002

## Dependency justification

- #9002 — consumes the beta derivation function added by #9002 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- crates/demo-cycle/src/gamma.rs

## Acceptance criteria

- [ ] `bats tests/demo-cycle/gamma.bats` exits 0 after the change.
- [ ] `crates/demo-cycle/src/gamma.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cycle/gamma.bats
```
