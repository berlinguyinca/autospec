## Goal

Add `scripts/lint-issue.sh` that exits non-zero when an issue body fails the §3 quality contract.

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `scripts/validate.sh` exits 0 after adding the fixture files.
- [ ] `grep -c GOAL_VAGUE` returns 0 for this file.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md && echo OK
```

### Operator/full verification

```bash
bash scripts/validate.sh
```
