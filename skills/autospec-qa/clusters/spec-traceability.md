# Cluster: spec-traceability

Scope: extract every spec requirement and build the traceability matrix between
spec lines and implementation artifacts (code, tests, proof artifacts).

Inputs:
- The spec file(s) under audit (docs/specs/*.md, .turbo/specs/*.md).
- Current HEAD of the implementation.

Responsibilities:
- Enumerate spec requirements as `REQ_ID → spec_path:line` rows.
- Map each requirement to one of: implemented (file:line), test (file:line),
  proof artifact (artifact path), or **MISSING**.
- Emit `finding.category=spec_traceability` for every MISSING row.
- Defer per-control UI audits to `functional-coverage`.
- Defer proof-artifact freshness to `reliability-contract`.

Output JSON shape (per finding):
```json
{
  "cluster": "spec-traceability",
  "category": "missing_requirement|stale_spec|contradiction",
  "req_id": "REQ_…",
  "spec_path": "docs/specs/foo.md:42",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh --category
spec_mismatch` before emitting.

TODO: backfill from `## Spec contradiction detector` + `## Spec supersession
(recency)` sections of SKILL.md.
