# Cluster: production-incidents

Scope: incident registry regression check (PR #661). Every recorded
production incident must have a regression test that would catch its
recurrence, and the test must be exercised this run.

Inputs:
- Incident registry (`.autospec/incidents/*.md` or repo-level INCIDENTS.md).
- Test suite that maps to incident IDs.

Responsibilities:
- Walk the production-incident regression check (see SKILL.md
  `## Production incident regression check`).
- For every registered incident, confirm a regression test exists, is
  exercised this run, and would fail if the bug returned.

Output JSON shape:
```json
{
  "cluster": "production-incidents",
  "category": "missing_regression_test|test_not_run|test_does_not_fail_on_repro",
  "incident_id": "INC-2026-0001",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

TODO: backfill from `## Production incident regression check` section of
SKILL.md.
