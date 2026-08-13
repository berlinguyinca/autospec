## Goal

Add AC_NOT_CHECKABLE enforcement to scripts/lint-issue.sh.

## Files to read first

- scripts/lint-issue.sh

## Implementation outline

1. Reject acceptance criteria without concrete tokens.

## Tests required

- bats tests/unit/test_lint_issue.bats

## Dependencies

none

## Acceptance criteria

- [ ] The generated issue is verifiable.

## Verification

### Primary smoke test

```bash
bats tests/unit/test_lint_issue.bats
```
