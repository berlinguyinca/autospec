# Boundary guardrails

Run the deterministic boundary scan before declaring an external integration
complete:

```bash
bash scripts/autospec-boundary-guardrails.sh scan --repo-root .
```

The JSON report contains findings for four independent failure classes:

- `CONTRACT_DRIFT` — a Python allow-list value is absent from a SQL `CHECK`.
- `SILENT_FAILURE` — an error branch returns an empty result without a log or
  warning.
- `BOUNDARY_TEST_MISSING` — typed fakes cover a decoder without a raw payload
  boundary test.
- `REAL_RESPONSE_EVIDENCE_MISSING` — an integration marked `area:integration`
  and `status: done` has no replayable fixture under `tests/fixtures/`,
  `recordings/`, or a `.har`/real-response artifact.

The scan is read-only and does not require network access or credentials. A
finding is a review signal: confirm the boundary, add the missing evidence, or
document a narrowly scoped suppression before merging.
