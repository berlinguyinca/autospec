## Goal

Add `tests/demo-ui/persist-roundtrip.bats` that round-trips a saved session through `persist.rs`.

## Files to read first

- tests/demo-ui/persist-roundtrip.bats
- crates/demo-ui/src/persist.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/persist-roundtrip.bats

## Dependencies

Depends on issue #2008

## Dependency justification

- #2008 — round-trips the JSON codec in `crates/demo-ui/src/persist.rs` (verification-requires-predecessor).

## Files touched

- tests/demo-ui/persist-roundtrip.bats

## Acceptance criteria

- [ ] `bats tests/demo-ui/persist-roundtrip.bats` exits 0 after the change.
- [ ] `tests/demo-ui/persist-roundtrip.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/persist-roundtrip.bats
```
