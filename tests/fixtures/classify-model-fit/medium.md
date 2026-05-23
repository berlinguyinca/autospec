## Goal

Integrate the telemetry adapter with the existing autospec pipeline and wire up the JSON output format.

## Files to read first

- `scripts/gen-issue-skeleton.sh`
- `scripts/lint-issue.sh`
- `tests/gen-issue-skeleton.bats`
- `.autospec/telemetry/` (existing format)

## Implementation scope

- `scripts/telemetry-adapter.sh`
- `tests/telemetry-adapter.bats`

## Acceptance criteria

- [ ] `bats tests/telemetry-adapter.bats` passes
- [ ] JSON lines appended to `.autospec/telemetry/adapter.jsonl`
