## Goal

Add `crates/demo-cycle/src/alpha.rs` that computes the alpha checksum for input bytes.

## Files to read first

- crates/demo-cycle/src/alpha.rs
- crates/demo-cycle/src/delta.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cycle/alpha.bats

## Dependencies

Depends on issue #9004

## Dependency justification

- #9004 — consumes the delta derivation function added by #9004 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- crates/demo-cycle/src/alpha.rs

## Acceptance criteria

- [ ] `bats tests/demo-cycle/alpha.bats` exits 0 after the change.
- [ ] `crates/demo-cycle/src/alpha.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cycle/alpha.bats
```
