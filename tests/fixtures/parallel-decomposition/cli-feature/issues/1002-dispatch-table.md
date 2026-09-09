## Goal

Add `crates/demo-cli/src/dispatch.rs` that routes parsed subcommands to handlers through a `DispatchTable`.

## Files to read first

- crates/demo-cli/src/dispatch.rs
- crates/demo-cli/src/config.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/dispatch.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/demo-cli/src/dispatch.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/dispatch.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/dispatch.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/dispatch.bats
```
