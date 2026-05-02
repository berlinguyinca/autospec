## Goal

Should probably refactor `scripts/foo.sh` to improve handling.

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `scripts/validate.sh` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md && echo OK
```
