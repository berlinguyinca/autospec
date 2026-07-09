# AutoSpec Growth — candidate pipeline (Plan 2 of 5)

**Status:** Design
**Date:** 2026-07-08
**Author:** berlinguyinca
**Depends on:** Plan 1 foundation (`growth-ledger.sh`, `growth-source-weights.sh`)
**Feeds:** `/autospec-grow-define` (Plan 3)

## Summary

The 6 growth researcher lenses are inherently LLM research work (crawl a site,
mine keywords, find on-topic communities). Their **surrounding pipeline is
deterministic** and therefore testable, so this plan builds exactly that spine
— four `autospec-grow-shared` bash/jq primitives, same model as Plan 1 — while
the lenses' actual research prompts and the LLM judgment calls live in the
`/autospec-grow-define` skill (Plan 3). This mirrors `autospec-explore`, where
scripts are the scaffolding and skill prose is the research.

The pipeline turns raw lens candidates into a ranked, de-duplicated, verified
work-list ready for decomposition into GitHub issues:

```
lens candidates → validate → dedup-against-ledger → LLM verify → verify-harness
                → rank (source-weighted, severity-first) → top-N → decompose (Plan 3)
```

## Components

All are `set -euo pipefail`, bash 3.2 compatible, under
`skills/autospec-shared/scripts/`, with bats suites under `tests/unit/`.

### 1. Candidate schema + validator

- `schemas/growth-candidate.schema.json`
- `scripts/validate-growth-candidate.sh <candidate.json>` — exit 0 valid,
  non-zero + stderr reason invalid. Fail-closed.

**Candidate record:**

```json
{
  "lens": "keyword-gap",           // one of the 6 sources
  "channel": "content",            // technical_seo|content|outreach|directories
  "kind": "artifact",              // artifact | outbound
  "title": "Add /vs/competitor comparison page",
  "norm_title": "add vs competitor comparison page",
  "rationale": "GSC shows 'x vs y' at position 12; competitor ranks 3",
  "evidence": ["gsc:query=x vs y,pos=12", "serp:competitor#3"],
  "roi": 4,                        // integer 1..5
  "effort": "medium",              // small | medium | large
  "severity": 3,                   // integer 1..5
  "confidence": 0.7                // number 0..1
}
```

Validated: required fields present; `lens` ∈ {technical-seo, keyword-gap,
content-opportunity, community, directory, backlink}; `kind` ∈ {artifact,
outbound}; `channel` ∈ the four channels; `roi`/`severity` integers 1..5;
`effort` ∈ {small, medium, large}; `confidence` number 0..1. Anything else →
reject.

### 2. Dedup-against-ledger

- `scripts/growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>` — emits
  the surviving candidates (JSONL) whose `norm_title` does **not** already
  appear in the ledger.

Dedup is against the **full seen-set** — every ledger line regardless of
outcome (`pending`, `merged_clean`, `published`, `rejected`, `refuted`,
`failed`) — so a refuted or rejected idea never resurfaces next cycle. (This is
the dedup-vs-seen-not-just-confirmed convergence rule; deduping only against
shipped items makes rejected candidates reappear every round.) Matching is exact
on the caller-normalized `norm_title` (string equality, never regex — no
metacharacter injection).

### 3. ROI/severity ranker

- `scripts/growth-candidate-rank.sh <candidates.jsonl>` — emits candidates
  sorted descending by a deterministic `rank_score`, with the score attached.

```
roi_norm      = roi / 5
severity_norm = severity / 5
effort_factor = { small: 1.0, medium: 0.7, large: 0.4 }
source_weight = growth-source-weights.sh[lens]      # Plan 1; empty ledger → 0.5
rank_score = (roi_norm * W_ROI + severity_norm * W_SEV)
             * confidence * effort_factor * source_weight
```

`W_ROI = 0.5`, `W_SEV = 0.5`. **Severity-first tiebreak:** equal `rank_score` →
higher `severity` wins, then higher `roi`. Source-weighting means lenses whose
past proposals shipped clean (per the ledger) automatically rank higher, and
lenses the verify stage keeps refuting are down-weighted — the recursive
self-improvement loop.

### 4. Verify-harness

- `scripts/growth-candidate-verify.sh <candidate.json> <verdict.json>` — the
  deterministic recorder for the LLM adversarial-verify verdict.

Verdict shape: `{"real": true|false, "reason": "..."}`.

- `real == true` → emit the candidate unchanged on stdout (it proceeds to
  ranking / filing).
- `real == false` → append a `refuted` line to the ledger
  (`growth-ledger.sh --append` with `issue:0`, `source:<lens>`,
  `outcome:"refuted"`, the reason) and emit nothing. Refuted candidates feed
  per-source down-weighting and never become issues.
- **Fail-closed:** a missing/unparseable verdict, or `real` absent/non-boolean,
  is treated as **refuted** (recorded reason `"unparseable verdict, refused"`).
  A candidate is never filed on ambiguous judgment.

### 5. `validate.sh` wiring

- `check_growth_candidate_pipeline_contract` — `bash -n` on the four new scripts
  + runs their four bats suites; enumerated in `main`'s run list (net-new
  suites, gate-atomicity rule). Registered next to
  `check_growth_shared_contract`.

## Reuse

- `growth-ledger.sh` (Plan 1) — verify-harness appends `refuted`; dedup reads it.
- `growth-source-weights.sh` (Plan 1) — ranker's per-lens weight.
- No new dependencies; no live API calls (those are Plan 4).

## Error handling

Fail-closed throughout: missing input files → non-zero; malformed JSON → non-zero
(never silently empty); unparseable verify verdict → refute (never file). Every
verify decision is auditable via the ledger line it writes.

## Testing (validation shell scripts, per repo convention)

- **Schema/validator:** valid candidate passes; each enum violation, out-of-range
  `roi`/`severity`, bad `effort`, missing required field → rejected; malformed
  JSON → rejected.
- **Dedup:** a candidate whose `norm_title` matches a `merged_clean` ledger line
  is dropped; one matching a `refuted`/`rejected` line is **also** dropped (full
  seen-set); a novel `norm_title` survives; empty ledger → all survive.
- **Ranker:** ordering matches the formula; severity-first tiebreak on equal
  score; a lens with higher source-weight (seeded via a real `--append` ledger
  built by `growth-ledger.sh`, not a bespoke fixture) ranks its candidate above
  an equal candidate from a low-weight lens.
- **Verify-harness:** `real:true` → candidate emitted, no ledger write;
  `real:false` → nothing emitted + exactly one `refuted` ledger line;
  unparseable verdict → refuted (fail-closed) + ledger line.
- **Integration:** feed a small candidate set through validate → dedup → verify
  → rank end-to-end; assert the surviving ranked order. Build ledger state via
  the real `growth-ledger.sh --append` (never a self-consistent fixture that
  could mask a schema mismatch — the Plan 1 cadence bug was exactly that).
- **`validate.sh`:** `check_growth_candidate_pipeline_contract` passes; full
  suite stays green.

## Open questions

- Ranker `W_ROI`/`W_SEV` default split (0.5/0.5) — revisit once real cycles show
  whether severity or ROI is the better predictor of shipped impact (the ledger
  will tell us; tune in a later pass, not now).
- Top-N cutoff for decomposition is a Plan 3 concern (the define skill decides
  how many ranked candidates to file per cycle), not part of this pipeline.
