# Cluster: backend-integration

Scope: API contracts, no-mock smoke, live-backend triage. Asserts that the
deployed app actually talks to the deployed backend with no mock seam.

Inputs:
- Deployed app URL.
- Backend service contract (OpenAPI / GraphQL schema / route table).
- No-mock smoke harness (see SKILL.md `## No-mock deployed smoke rule`).

Responsibilities:
- Enumerate API routes the app calls; for each, prove a live round-trip exists.
- Detect mock seams that survive into the deployed bundle.
- Triage live-backend blockers via the prompt in SKILL.md
  `## Live backend blocker triage prompt`.
- Defer proof-artifact storage to `reliability-contract`.

Output JSON shape:
```json
{
  "cluster": "backend-integration",
  "category": "mock_seam_in_prod|missing_route_proof|contract_drift",
  "route": "POST /api/users",
  "file": "src/api/users.ts:42",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category missing_function` or `regression`).

TODO: backfill from `## No-mock deployed smoke rule` +
`## No-mock deployed smoke blocker handling` +
`## Live backend blocker triage prompt` sections of SKILL.md.
