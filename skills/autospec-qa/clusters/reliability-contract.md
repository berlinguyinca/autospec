# Cluster: reliability-contract

Scope: proof matrix, control ledger, mutation proof, canary, no-mock minimum
coverage, observability contract, data-lifecycle proof.

Inputs:
- Proof artifact directory (`.autospec/proof/` or equivalent).
- Mutation-testing harness output.
- Canary deployment evidence (post-merge).

Responsibilities:
- Walk every spec-mandated proof artifact and assert freshness (see SKILL.md
  `## Artifact freshness gate` + `## Evidence provenance gate`).
- Run the mutation proof gate (see SKILL.md
  `## Mutation and breakage proof prompt`).
- Enforce no-mock minimum coverage (see SKILL.md
  `## No-mock minimum coverage prompt`).
- Enforce observability contract + data-lifecycle proof.
- Defer legacy-removal checks to `legacy-and-cleanup`.

Output JSON shape:
```json
{
  "cluster": "reliability-contract",
  "category": "stale_proof|missing_mutation_proof|coverage_floor|missing_canary",
  "artifact": ".autospec/proof/foo.json",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

TODO: backfill from `## Proof artifact requirements`,
`## Artifact freshness gate`, `## Evidence provenance gate`,
`## Mutation and breakage proof prompt`, `## Post-merge deployed canary prompt`,
`## No-mock minimum coverage prompt`, `## Observability contract`,
`## Data lifecycle proof` sections of SKILL.md.
