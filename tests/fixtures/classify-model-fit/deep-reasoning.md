## Goal

Design and architect the anonymization engine for the clone pipeline, deciding on the reversible-map strategy and reconciling PII detection approaches across database drivers.

## Files to read first

- `docs/specs/2026-05-22-autospec-e2e-clone-design.md` §5
- `skills/autospec-e2e-clone/scripts/load-contract.sh`
- `schemas/autospec-clone-contract.schema.json`

## Implementation scope

- `skills/autospec-e2e-clone/scripts/anonymize.sh`

## Acceptance criteria

- [ ] `bats skills/autospec-e2e-clone/tests/anonymize.bats` passes
