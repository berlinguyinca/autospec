## Goal

Add `tests/demo-cli/output-snapshots.bats` that snapshots rendered stdout for both `--format` styles.

## Files to read first

- tests/demo-cli/output-snapshots.bats
- crates/demo-cli/src/output.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/output-snapshots.bats

## Dependencies

Depends on issue #1003

## Dependency justification

- #1003 — snapshots stdout produced by `render` in `crates/demo-cli/src/output.rs` (verification-requires-predecessor).

## Files touched

- tests/demo-cli/output-snapshots.bats

## Acceptance criteria

- [ ] `bats tests/demo-cli/output-snapshots.bats` exits 0 after the change.
- [ ] `tests/demo-cli/output-snapshots.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/output-snapshots.bats
```
