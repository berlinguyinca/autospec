## Goal

Add `scripts/lint-issue.sh` that exits non-zero when an issue body fails the §3 quality contract.

## Files to read first

- scripts/lint-issue.sh
- scripts/validate.sh

## Implementation outline

1. Parse the `--json` and `--help` flags plus the body-file argument.
2. Extract each `## ` section and apply the per-rule checks.
3. Accumulate findings and exit with their count.

## Tests required

- bats tests/unit/test_lint_issue.bats

## Dependencies

Depends on issue #152

## Files touched

- scripts/lint-issue.sh

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `scripts/validate.sh` exits 0 after adding the fixture files.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/<TODO>
```
