## Goal

Add `crates/demo-cli/src/output.rs` that renders command results to stdout in the selected `--format` style.

## Files to read first

- crates/demo-cli/src/output.rs
- crates/demo-cli/src/dispatch.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/output.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-cli/src/output.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/output.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/output.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/output.bats
```
