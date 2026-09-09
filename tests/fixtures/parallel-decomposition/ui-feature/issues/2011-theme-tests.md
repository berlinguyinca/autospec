## Goal

Add `tests/demo-ui/theme.bats` that asserts the theme tokens render the 4 required palette colors.

## Files to read first

- tests/demo-ui/theme.bats
- crates/demo-ui/src/theme.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-ui/theme.bats

## Dependencies

Depends on issue #2004

## Dependency justification

- #2004 — asserts on the palette tokens defined in `crates/demo-ui/src/theme.rs` (verification-requires-predecessor).

## Files touched

- tests/demo-ui/theme.bats

## Acceptance criteria

- [ ] `bats tests/demo-ui/theme.bats` exits 0 after the change.
- [ ] `tests/demo-ui/theme.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-ui/theme.bats
```
