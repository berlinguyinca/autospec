## Goal

Add `tests/telemetry/metrics.bats` that asserts the exporter emits the 5 declared counter names.

## Files to read first

- tests/telemetry/metrics.bats
- crates/telemetry/src/metrics.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/telemetry/metrics.bats

## Dependencies

Depends on issue #3004

## Dependency justification

- #3004 — asserts on counter names emitted by the `Metrics` API in `crates/telemetry/src/metrics.rs` (verification-requires-predecessor).

## Files touched

- tests/telemetry/metrics.bats

## Acceptance criteria

- [ ] `bats tests/telemetry/metrics.bats` exits 0 after the change.
- [ ] `tests/telemetry/metrics.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/telemetry/metrics.bats
```
