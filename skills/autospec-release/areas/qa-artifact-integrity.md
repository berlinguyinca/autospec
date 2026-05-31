# Area: qa-artifact-integrity

Verify `.autospec/proof-matrix.json`, control-ledger, mutation-proof, and
canary-results are fresh and schema-valid.

## Scope

- `.autospec/proof-matrix.json`
- `.autospec/control-ledger.json`
- `.autospec/mutation-proof.json`
- `.autospec/canary-results.json`
- `.autospec/qa-verdict.json` (consumed by `compute-release-verdict.sh`)

## Checks

- Every artifact's `head_sha` field matches current HEAD.
- Schema validation via
  `skills/autospec-shared/scripts/validate-qa-artifacts.sh` when available.
- `live_app_proof: true` in qa-verdict.json — mocked-only proofs fail this
  area.
- Control-level evidence covers text boxes, selects, dropdowns, buttons,
  validation banners, API effects, fallback behavior, and accessibility.

## Findings shape

- `area: qa-artifact-integrity`
- `status`, `release_blocking`, `summary`, `evidence`.
- `release_blocking: true` for stale/missing artifacts or `live_app_proof:
  false`.

## PASS criteria

- All four artifacts present, fresh, and schema-valid.
- qa-verdict.json's `head_sha` == current HEAD AND `live_app_proof: true`.
- No `no_mock_smoke` or `live_backend_blocker` findings.

## Token budget

Tier B reviewer; 50-120k context; 25 tool calls cap.
