# LLM touchpoint decision table

Source: `docs/specs/2026-06-04-skill-token-efficiency-design.md` §4.

## How to re-run the quality differential

```bash
bash scripts/quality-differential.sh \
  --step refine-lenses \
  --fixtures tests/fixtures/quality-diff/refine-lenses
```

Each fixture directory ships an `assert.sh` (signal-property checks, not string
equality).  Exit non-zero means the deterministic path is below quality bar and
the step must keep its LLM path.

## Default conversion rule

**KEEP-LLM-WITH-ESCALATION** unless a quality-differential fixture run proves
deterministic output ≥ LLM output for that step (per §4 and §7 of the design
spec).  A failing fixture verdict is sufficient to block conversion; a passing
verdict must cover ≥ 3 independent fixtures before a determinization PR is
accepted.

## Decision table

| Touchpoint | Current impl | Verdict | Rationale | Fixture path |
|---|---|---|---|---|
| Phase 1 research | LLM (Tier A, 25-call cap) | KEEP-LLM | Open-ended exploration; no deterministic substitute | `tests/fixtures/quality-diff/refine-lenses/` — any future conversion attempt must pass these fixtures (INVERT precedent) |
| Phase 2 brainstorm/design | LLM (orchestrator) | KEEP-LLM | Creative spec synthesis; output varies per prompt intent | `tests/fixtures/quality-diff/refine-lenses/` — harness any future conversion must pass (per-row evidence) |
| Phase 3 decompose | LLM (Tier A) + `lint-issue.sh` | KEEP-LLM (det lint retained) | Semantic spec→issue mapping requires open-ended reasoning; deterministic lint retained as a guard | `tests/fixtures/quality-diff/refine-lenses/` — harness any future conversion must pass |
| Phase 3.5 classify | deterministic-first (`classify-model-fit.sh`) | DONE (#421) | LLM only on ambiguity; deterministic path proven sufficient for the unambiguous majority | `scripts/classify-model-fit.sh` + `tests/unit/test_listener_classify.bats` |
| Phase 4 implementer | LLM (Tier B) + `lint-implementation.sh` | KEEP-LLM (det lint retained) | Code authoring; deterministic lint retained as a pre-review gate | `tests/fixtures/quality-diff/refine-lenses/` — harness any future conversion must pass |
| Phase 4 guardian+LGTM | hybrid (det RULE_IDs + LLM semantic) | KEEP-HYBRID | Working as designed; det checks catch mechanical violations, LLM catches semantic gaps | `tests/fixtures/quality-diff/synth/` — synthetic step fixtures (`fail-canned`, `pass-signal`) validate the harness itself |
| Phase 5.5 gap audit | LLM (Tier A) | KEEP-LLM | Cross-issue integration reasoning; no deterministic substitute | `tests/fixtures/quality-diff/refine-lenses/` — harness any future conversion must pass |
| refine lenses | deterministic boilerplate + LLM hatch | **INVERT: LLM-first / det-assist** | Fixture-proven: deterministic path is below quality bar on all three `refine-lenses` fixtures (`caching-profile-api`, `csv-export-stream`, `oauth-token-refresh`); implemented via `AUTOSPEC_REFINE_LENS_MODE` env hatch (PR #1031) | `tests/fixtures/quality-diff/refine-lenses/caching-profile-api/`, `tests/fixtures/quality-diff/refine-lenses/csv-export-stream/`, `tests/fixtures/quality-diff/refine-lenses/oauth-token-refresh/` |
