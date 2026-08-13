## Goal

Exclude autospec generated metadata from scripts/lint-issue.sh word budgets.

## Files to read first

- scripts/lint-issue.sh

## Implementation outline

1. Strip marker-bounded generated sections before counting words.

## Tests required

- bats tests/unit/test_lint_issue.bats

## Dependencies

none

## Acceptance criteria

- [ ] scripts/lint-issue.sh exits 0 for good-generated-metadata.md.

## Verification

### Primary smoke test

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good-generated-metadata.md
```

## Model fit

<!-- autospec-classify:begin -->
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
<!-- autospec-classify:end -->

## Shared contracts

<!-- autospec-shared-contracts:begin -->
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
generated generated generated generated generated generated generated generated generated generated
<!-- autospec-shared-contracts:end -->
