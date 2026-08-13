## Goal

Require Primary smoke test in scripts/lint-issue.sh.

## Files to read first

- scripts/lint-issue.sh

## Implementation outline

1. Reject an absent smoke subsection.

## Tests required

- bats tests/unit/test_lint_issue.bats

## Dependencies

none

## Acceptance criteria

- [ ] SMOKE_NOT_FENCED is emitted 1 time for this fixture.
