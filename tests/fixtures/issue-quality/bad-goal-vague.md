## Goal

Improve the decomposer prompt for better issue quality.

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `scripts/validate.sh` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md && echo OK
```
