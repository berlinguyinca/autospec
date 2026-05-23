## Goal

Design and reconcile the cross-skill autospec-e2e-clone pipeline with the existing autospec-test adapter row, resolving schema conflicts and redesigning the contract shape to support multi-skill orchestration.

## Files to read first

- `docs/specs/2026-05-22-autospec-e2e-clone-design.md` §2
- `docs/specs/2026-05-22-autospec-e2e-clone-design.md` §11
- `skills/autospec-test/scripts/load-contract.sh` §3
- `schemas/autospec-test-contract.schema.json`
- `skills/autospec-shared/scripts/` (cross-skill shared)
- `docs/specs/2026-05-22-autospec-tooling-optimization-design.md` §4
- `scripts/validate.sh`
- `tests/fixtures/gen-issue-skeleton/`

## Implementation scope

- `skills/autospec-e2e-clone/SKILL.md`
- `skills/autospec-e2e-clone/scripts/load-contract.sh`
- `schemas/autospec-clone-contract.schema.json`

## Acceptance criteria

- [ ] `ajv compile -s schemas/autospec-clone-contract.schema.json`
- [ ] `bash scripts/validate.sh` passes
