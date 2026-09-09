## Goal

Add `tests/demo-cli/help-coverage.bats` that asserts every registered subcommand appears in the usage text.

## Files to read first

- tests/demo-cli/help-coverage.bats
- crates/demo-cli/src/help.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/help-coverage.bats

## Dependencies

Depends on issue #1005

## Dependency justification

- #1005 — asserts on the usage text printed by `print_usage` in `crates/demo-cli/src/help.rs` (verification-requires-predecessor).

## Files touched

- tests/demo-cli/help-coverage.bats

## Acceptance criteria

- [ ] `bats tests/demo-cli/help-coverage.bats` exits 0 after the change.
- [ ] `tests/demo-cli/help-coverage.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/help-coverage.bats
```
