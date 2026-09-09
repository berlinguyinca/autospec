## Goal

Add `tests/telemetry/logging.bats` that asserts log lines parse as 3-field JSON records.

## Files to read first

- tests/telemetry/logging.bats
- crates/telemetry/src/logging.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/telemetry/logging.bats

## Dependencies

Depends on issue #3003

## Dependency justification

- #3003 — asserts on JSON records emitted by the `Logger` facade in `crates/telemetry/src/logging.rs` (verification-requires-predecessor).

## Files touched

- tests/telemetry/logging.bats

## Acceptance criteria

- [ ] `bats tests/telemetry/logging.bats` exits 0 after the change.
- [ ] `tests/telemetry/logging.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/telemetry/logging.bats
```
