## Goal

Add `crates/demo-cli/src/wiring.rs` that passes parsed `DemoConfig` values into the `DispatchTable`.

## Files to read first

- crates/demo-cli/src/wiring.rs
- crates/demo-cli/src/config.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/demo-cli/wiring.bats

## Dependencies

Depends on issue #1001
Depends on issue #1002

## Dependency justification

- #1001 — consumes the `DemoConfig` type added by #1001 (consumes-new-type), which does not exist on the current base branch.
- #1002 — consumes the `DispatchTable` interface added by #1002 (consumes-new-interface), which does not exist on the current base branch.

## Files touched

- crates/demo-cli/src/wiring.rs

## Acceptance criteria

- [ ] `bats tests/demo-cli/wiring.bats` exits 0 after the change.
- [ ] `crates/demo-cli/src/wiring.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/demo-cli/wiring.bats
```
