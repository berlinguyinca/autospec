## Goal

Add scripts/validate-query.sh to reject write-capable database statements.

## Source spec

`docs/specs/read-only-query-design.md` — https://example.com/read-only-query-design

## Team personality

- Security delivery: database owner, application engineer, test engineer

## Review counter-team

- Adversarial review: red team, SRE, data-governance reviewer

## Files to read first

- docs/specs/read-only-query-design.md
- scripts/validate-query.sh

## Files touched

- scripts/validate-query.sh
- tests/security-query.bats

## Local-LLM execution notes

- Treat the database grant as authoritative and preserve every listed control.

## Dependencies

Depends on issue #41

## Evidence consumed

- E1 verified: replica availability from scripts/probe-replica.sh

## Controls covered

- T1: SELECT-only database role prevents data loss

## Prerequisites

- P1 verified: read replica is available

## Implementation scope

- scripts/validate-query.sh and grant-level rejection tests

## Out of scope

- Write access

## Implementation outline

1. Add the statement validator
2. Exercise grants without the validator

## Tests required

- bats tests/security-query.bats against a real database fixture

## Acceptance criteria

- [ ] scripts/validate-query.sh rejects `INSERT` with exit 1
- [ ] tests/security-query.bats proves 6 grant-level writes fail

## Verification

### Primary smoke test

```
bats tests/security-query.bats
```

### Operator full

```
bash scripts/validate-query.sh --self-test
```

## Branch name

`feat/security-query-validator`
