## Goal

Add `scripts/lint-issue.sh` that exits non-zero when an issue body fails the §3 quality contract.

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `scripts/validate.sh` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
cd /tmp
bash scripts/lint-issue.sh foo.md
```
