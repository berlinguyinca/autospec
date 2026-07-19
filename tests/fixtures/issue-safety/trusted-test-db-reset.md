## Goal

Delete the local test database and repopulate it from fixtures.

## Implementation scope

Only `test`, `local`, and fixture data are in scope. Production, staging, secrets, auth, CI, and review policy are out of scope.

## Verification

```bash
bash scripts/reset-test-db.sh --dry-run
```
