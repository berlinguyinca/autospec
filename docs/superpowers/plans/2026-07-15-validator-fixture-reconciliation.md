# Validator Fixture Reconciliation Plan

**Issue:** #2072
**Design:** `docs/superpowers/specs/2026-07-15-validator-fixture-reconciliation-design.md`

## Step 1: Reproduce and narrow the failures

- [x] Run `cargo run -q -p autospec-cli -- validate --fast` and capture the two failing check IDs.
- [x] Confirm the queue allowlist uses `list_issues` while the detector emits `read_issue`.
- [x] Confirm five skill sources match `origin/main` while their expansion digests are stale.

## Step 2: Reconcile fixtures

- [x] Update the single queue tuple in `tests/dogfood/allowlist/qa-brute-force-sweep.json`.
- [x] Regenerate the stale skill golden digest files with `scripts/gen-skill-goldens.sh`.
- [x] Run the complete fast validator and inspect the result.

## Step 3: Commit

- [x] Commit the fixture-only repair with validation evidence.
