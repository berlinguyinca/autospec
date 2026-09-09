## Goal

Add `crates/demo-cli/src/config.rs` that parses `--config` arguments into the `DemoConfig` struct.

## Files to read first

- crates/demo-cli/src/config.rs
- crates/demo-cli/src/main.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/config.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-cli/src/config.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/config.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/config.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/config.bats
```
