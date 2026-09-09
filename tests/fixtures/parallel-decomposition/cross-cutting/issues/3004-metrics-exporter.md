## Goal

Add `crates/telemetry/src/metrics.rs` that exports counters through a bounded-cardinality `Metrics` API.

## Files to read first

- crates/telemetry/src/metrics.rs
- crates/contracts/src/lib.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/telemetry/metrics-exporter.bats

## Dependencies

none

## Dependency justification

none

## Files touched

- crates/telemetry/src/metrics.rs

## Acceptance criteria

- [ ] `bats tests/telemetry/metrics-exporter.bats` exits 0 after the change.
- [ ] `crates/telemetry/src/metrics.rs` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/telemetry/metrics-exporter.bats
```
