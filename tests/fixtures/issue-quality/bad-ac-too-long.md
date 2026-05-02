## Goal

Add `scripts/lint-issue.sh` that exits non-zero when an issue body fails the §3 quality contract.

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0 with no output on stderr and the command completes in under 5 seconds and also handles edge cases properly.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md && echo OK
```
