# autospec cost efficiency — token reporting, subagent-per-issue, prefix slim, tier right-sizing

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (brainstormed with Claude; empirical data from live runs)
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

Autospec works, but burns far more tokens per issue than the work requires.
Empirical (this session's runs): implementer subagents 60k–160k tokens/issue,
reviewers 40–50k, decomposers 50–75k. Investigation (2026-06-03, file:line
verified) found the cost is dominated by avoidable overhead:

1. **~29k tokens of static prefix per implementer dispatch** — the whole 70KB
   `skills/autospec-run/SKILL.md` is injected verbatim (mostly monitor-loop
   bash the implementer never runs), plus AGENTS.md, **plus a duplicate
   RULE_ID table** re-extracted from the AGENTS.md just emitted
   (`bundle-static-context.sh:168-176`).
2. **Prompt cache built but poisoned/dormant** — `<!-- CACHE BOUNDARY -->`
   exists (`bundle-static-context.sh:156,236`), but issue-label-tagged memory
   is injected *inside* the boundary (`:120-152`) so prefix bytes differ per
   issue (guaranteed miss); the roadmap spec flags cache_control as "built but
   unused by the current launch path".
3. **Amplification** — `MAX_IMPL_RETRIES=5` × ≤3 reviewer iterations; the
   issue body is fetched ≥4×/issue (`autospec-run SKILL.md:551,633,650,855`);
   PR diff re-fetched per reviewer iteration.
4. **Tier overkill** — Phase 3.5 labeling and /autospec-classify mandate
   Tier A (opus+ultrathink) for mostly-mechanical classification; regression
   issues dispatch a *second* Tier-A meta-review subagent.
5. **Telemetry is dead code** — `record-telemetry.sh` + schema exist but
   nothing writes the `tokens-<ISSUE>.json` it waits for, and no token data
   ever reaches GitHub.
6. **The new autospec-doc feature multiplies LLM calls** — validator-retry ×
   ai-review, per page × per audience × per feature, full-regen default.

## Goals (operator-decided)

- **G1 — Per-issue token reporting:** after each issue merges, post ONE issue
  comment with the token breakdown (implementer / reviewer / recovery, model,
  total, PR#) AND append the same data to `~/.autospec/telemetry.jsonl` via
  the existing `record-telemetry.sh`. Harness-neutral best-effort.
- **G2 — Fresh-subagent-per-issue is canonical:** every issue is processed in
  a NEW top-level subagent; the orchestrator/main agent never implements
  in-context. `AUTOSPEC_BATCH_SIZE` default 3 → **1**.
- **G3 — Prefix slim (curated implementer-contract):** replace whole-SKILL.md
  injection with a small dedicated contract doc; fix the cache boundary.
- **G4 — Tier right-sizing:** Phase 3.5/classify → deterministic-first +
  Tier B on ambiguity; regression meta-review folded into the single reviewer
  pass; fused guardian+LGTM reviewer → Tier B **always** (incl. priority:high),
  with `AUTOSPEC_REVIEWER_TIER` env override (set `opus` to restore Tier A) as
  the one-variable escape hatch.
- **G5 — Duplicate-read elimination:** fetch the issue body ONCE per issue
  into a temp file; all later steps consume the file. Reviewer reuses the
  first iteration's diff unless the branch changed (compare head SHA).
- **G6 — Doc-feature cost caps:** deterministic validator runs FIRST and gates
  the LLM retry loop (no LLM regen for deterministic failures); ONE batched
  ai-review call per audience (not per section); `--full` stays sweep-only and
  the orchestrator's default path is incremental (changed scopes only).

**Non-goals:** weakening correctness gates (validate.sh, docs-as-tests
blocking, rebase-and-retest, lock-step) — retries stay capped at 5/3 but each
retry gets cheaper; no changes to merge authority or queue semantics.

## Design

### D1 — Token reporting (G1)

- **Producer:** the orchestrator (which receives subagent token usage in the
  Agent/task result — verified live) writes
  `.autospec/tokens-<ISSUE>.json` in the schema `record-telemetry.sh` already
  expects (`{input_tokens, cache_creation_input_tokens,
  cache_read_input_tokens, output_tokens}` per role; fields best-effort —
  absent fields recorded as null, never blocking).
- **Recorder:** the existing dormant guards in the run trio
  (`SKILL.md:822,896`) finally fire; rows land in `telemetry.jsonl`
  (existing schema, `AUTOSPEC_TELEMETRY_FILE` override preserved).
- **Sink:** new helper `skills/autospec-run/scripts/post-token-report.sh
  --issue N --repo R [--tokens-json F]` composes and posts ONE idempotent
  comment (marker `<!-- autospec-tokens:begin/end -->`, edit-in-place on
  re-run) after admin-merge, before `batch-done.json`. Trio prose gains the
  step at exactly that slot. Comment format:

  ```
  ## Token usage
  - implementer: 121,643 (opus) — PR #930
  - reviewer: 41,200 (sonnet)
  - recovery: — / total: 162,843
  <!-- autospec-tokens:begin -->…<!-- autospec-tokens:end -->
  ```
- Harness fallback: when usage is unavailable, post `tokens: unavailable on
  <harness>` once — never fail the run.

### D2 — Subagent-per-issue canonical (G2)

Exact edits (from the investigation): `AGENTS.md:399` default literal `3`→`1`;
all seven `${AUTOSPEC_BATCH_SIZE:-3}` defaults in the run trio → `:-1`;
rewrite the absorbed-discipline paragraph (`SKILL.md:265`, restated `:575`,
`:636-641`, lock-step mirrors) to state: **each issue is processed by a fresh
top-level subagent dispatched by the orchestrator; the orchestrator never
implements in its own context; batch>1 is an explicit operator opt-in**
(`AUTOSPEC_BATCH_SIZE=N`), retaining the reasoning:deep force-to-1 rule.

### D3 — Prefix slim + cache fix (G3)

- New `skills/autospec-run/prompts/implementer-contract.md` (target ≤24KB ≈
  6k tokens): the implementer-relevant extract ONLY — project rules, RULE_ID
  table (single copy), lock-step discipline, heartbeat schema, worktree/branch
  rules, retry/review loop contract, merge gate summary. The 70KB SKILL.md is
  NO LONGER injected into implementer dispatches.
- `bundle-static-context.sh`: emit `implementer-contract.md` + AGENTS.md
  *quality-contract sections* inside the boundary; DELETE the duplicate
  RULE_ID re-extraction; MOVE tag-filtered memory + per-issue scaffolding
  BELOW the closing `<!-- CACHE BOUNDARY -->` so the prefix is byte-stable
  across issues.
- Launch path passes the prefix with `cache_control: {type: ephemeral}` where
  the harness supports it (prose instruction in the trio; the reviewer path
  already reuses its prefix across inner-loop iterations).
- Guard: `validate.sh` named-content check — contract file exists, ≤24KB, and
  contains the RULE_ID table header; sweep flags drift between contract and
  AGENTS.md rule sections.

### D4 — Tier right-sizing (G4)

- AGENTS.md tier tables + Phase 3.5 / autospec-classify trios: classification
  runs the deterministic rubric first (file counts, verb keywords — per
  tracker #421's direction); only ambiguous issues get an LLM call, at
  **Tier B**. Sibling normalization stays deterministic.
- Run trio reviewer block: `TIER_B` for ALL issues including
  `regression`/`priority:high`; the second Tier-A regression meta-review
  dispatch is REMOVED — its "would the reviewer have caught the original
  gap?" check becomes a mandatory bullet in the single reviewer brief.
- Escape hatch: `AUTOSPEC_REVIEWER_TIER` (unset → sonnet; `opus` → Tier A)
  honored in the reviewer dispatch text; documented in AGENTS.md.

### D5 — Duplicate-read elimination (G5)

The run trio's process(ISSUE) fetches the issue body exactly once to
`/tmp/issue-<N>-body.md` (already half-done via `gen-implementer-prompt.sh
--issue-body`); the start-summary awk extraction, prompt assembly, drift-gate
grep, and reviewer prompt all read that file. Reviewer iterations >1 re-fetch
the PR diff only when the head SHA changed.

### D6 — Doc-feature cost caps (G6)

In `skills/autospec-doc/scripts/`:
- `gen-audience-docs.mjs`: run `defaultValidator` (deterministic scope-comment
  well-formedness) BEFORE any LLM validator; deterministic failures regen
  without an LLM verdict call; the LLM validator only adjudicates
  prose-quality retries.
- ai-review: ONE batched call per audience per generation run (sections
  concatenated with per-section confidence markers parsed out), replacing
  per-section calls.
- `doc-orchestrator.mjs`: default (bare) subcommand = incremental — changed
  scopes only (consume `check-doc-drift.sh --working-tree` to compute the
  set); full fan-out only under `--full`.

## Testing & validation

- bats: `post-token-report.sh` (compose, idempotent edit-in-place, missing
  tokens-json fallback); batch-default=1 assertions in the existing
  batch-size-gating tests; `AUTOSPEC_REVIEWER_TIER` plumbing.
- `.mjs` tests: deterministic-first gating (no LLM call on deterministic
  failure — assert call counts via the existing stub pattern); batched
  ai-review parse; incremental scope-set computation.
- Size guard test: `implementer-contract.md` ≤24KB.
- `validate.sh`: contract-file named-content check; updated lock-step trio
  expectations (run trio text changes in D1/D2/D4/D5 are trio edits — author
  SKILL.md, regenerate mirrors, byte-identical).
- Telemetry e2e: a fixture tokens-json → telemetry.jsonl row + composed
  comment body match golden files.

## Risks

| Risk | Mitigation |
|---|---|
| Contract doc drifts from SKILL.md/AGENTS.md | validate.sh named-content + sweep drift check |
| Reviewer quality drops on sonnet (high-stakes) | `AUTOSPEC_REVIEWER_TIER=opus` single-variable revert; Phase 5.5 audits watch for review misses |
| Cache still unused by some launch paths | savings from prefix slim (D3) are cache-independent; cache is upside |
| Trio conflicts with in-flight autospec-doc epic (#922-#924 edit the same run trio) | all run-trio-touching children depend on the doc epic's audit issue closing |

## Decomposition hint for /autospec-define

1. **Token reporting** (D1): `post-token-report.sh` + producer/recorder wiring
   + run-trio comment step. *(Trio toucher — depends on autospec-doc #925.)*
2. **Subagent-per-issue + batch=1** (D2): AGENTS.md + run-trio defaults +
   absorbed-discipline rewrite. *(Trio toucher — depends on #925 and on
   child 1 to serialize trio edits.)*
3. **implementer-contract.md + bundle-static-context slim + cache boundary
   fix** (D3): contract doc, dedupe, reorder, size-guard test, validate check.
4. **Tier right-sizing** (D4): AGENTS.md tables + 3.5/classify
   deterministic-first + reviewer block + env hatch. *(Touches run trio +
   classify trio — serialize after child 2.)*
5. **Duplicate-read elimination** (D5). *(Trio toucher — serialize after 4.)*
6. **Doc-feature cost caps** (D6): the three .mjs changes + tests. *(Depends
   on autospec-doc #918-#921 being merged; independent of the trio chain.)*
7. Standard Phase 5.5 audit issue depending on all.

> Decomposer notes: serialize ALL run-trio-touching children (1→2→4→5) with
> explicit `Depends on` edges AND on `#925`; lock-step discipline per trio
> edit; do NOT apply needs-autospec-template.
