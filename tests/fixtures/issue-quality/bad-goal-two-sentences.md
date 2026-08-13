## Goal

Add a gate to scripts/lint-issue.sh. Reject a second sentence.

## Files to read first

- scripts/lint-issue.sh

## Implementation outline

1. Count sentence terminals.

## Tests required

- bats tests/unit/test_lint_issue.bats

## Dependencies

none

## Acceptance criteria

- [ ] scripts/lint-issue.sh exits 1 for this fixture.

## Verification

### Primary smoke test

```bash
bats tests/unit/test_lint_issue.bats
```
