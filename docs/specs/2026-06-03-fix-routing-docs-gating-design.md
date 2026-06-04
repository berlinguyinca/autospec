# 'fix' intent routing + docs-generation gating (off-by-default in runs, always in QA)

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (refined via /autospec-refine)
- **Tracker target:** `berlinguyinca/autospec`
- **Builds on:** explore/discover routing (`2026-06-03-explore-discover-intent-routing-design.md`), autospec-doc core (`2026-06-03-autospec-doc-core-design.md` §D6), cost-efficiency (`2026-06-03-autospec-cost-efficiency-design.md`)

## Problem statement

Three operator-decided changes:

1. **`fix` is not a routed verb.** Imperative "fix the login flow" should enter
   the pipeline like implement/build/ship do — but routing to **`/autospec`
   end-to-end** (spec → issues → run), since a fix needs scoping before
   implementation. `new feature` already routes to `autospec-define`
   (plan-only) and **stays unchanged** (explicit decision).
2. **Doc generation currently fires unconditionally during runs.** The merged
   Phase-4 `regenerate` self-heal (PR #935) regenerates docs on every
   drift/missing_scope/example_stale finding; the only escape is per-issue
   `docs: skip`. Generation is expensive (audience × feature fan-out) — it
   must be **off by default during autospec runs** unless explicitly enabled.
   Drift **detection** stays on (cheap, informative labels).
3. **QA must always do docs.** `autospec-qa` becomes the guaranteed
   documentation sync point: it ALWAYS invokes `/autospec-doc --full`
   (generation + completeness audit + example verification), ignoring the
   off-by-default switch.

## Goals / non-goals

**Goals**
- G1: `fix` → `/autospec` via the autospec-listen deterministic classifier,
  intent-gated, with the standard one-line opt-out (`plain`/`no`) — no confirm
  gate (`/autospec` is not a perpetual loop) — plus **trivial-fix suppressors**.
- G2: `documentation.auto_regenerate: false` schema default; three opt-ins:
  (a) config `true`, (b) per-issue body line `docs: generate` (mirror of the
  existing `docs: skip`), (c) per-run flag `--with-docs`. Detection unchanged.
- G3: autospec-qa gains a mandatory **Documentation gate**: `/autospec-doc
  --full` + `verify-examples` always run; failures are QA findings (never
  silently skipped).

**Non-goals**
- Changing `new feature` routing (stays → autospec-define).
- Touching drift detection, labels, or `docs: skip` semantics.
- Weakening the docs-as-tests engine (QA runs it in full).

## Design

### D1 — `fix` routing (listener-match + listen trio)

- **D3 branch** in `scripts/listener-match.sh`: imperative `fix` (whole-word)
  → `emit_classify_json true autospec fix imperative 0.7 "$is_auto" "" ""`
  (no gate — standard opt-out applies; `/autospec` itself brainstorms scope).
  Place alongside the implement/build/ship branch; `fix` must NOT shadow the
  more specific explore/refine/run branches (evaluate after them).
- **Trivial-fix suppressors** (the negative table — false-negative biased):
  "fix this typo", "fix the typo", "fix the comment", "fix the indentation",
  "fix the formatting", "quick fix", "fix the spelling" → `match:false`, stay
  plain. Plus the existing D4 question/negation/past suppression ("did you fix
  it?", "don't fix that yet", "I fixed it").
- **Listen trio**: add the `fix → /autospec` row to the verb-map table
  (lock-step, byte-identical bodies).
- bats: positive table ("fix the login flow", "fix the race condition in the
  watchdog", "please fix the broken pagination") → `skill=autospec`,
  `intent=imperative`; negative table (all suppressors above) → `match:false`.

### D2 — docs off-by-default in runs

- **Config**: `documentation.auto_regenerate` (boolean, default **false**) in
  `skills/autospec-doc/scripts/doc-config.mjs` (#917's loader) + the schema
  docs + README migration note. `init` writes it explicitly with a comment.
- **Gate conditional**: the run trio's `docs-drift-gate` block (the #935
  self-heal) wraps the regenerate path in the resolved switch:
  `auto_regenerate==true` ∨ issue body matches `^docs:\s*generate\s*$`
  (case-insensitive, same matcher shape as `docs: skip`) ∨ run launched with
  `--with-docs` (plumbed as `AUTOSPEC_WITH_DOCS=1`). When OFF: detection runs,
  labels/comments still applied, classifier verdict logged, **no generation,
  no docs commit** — log line `docs: regeneration skipped (auto_regenerate=false)`.
  Precedence: `docs: skip` > `docs: generate` > config/flag.
- Phase 5.5 docs-completeness (#923) and sweep `--full` (#924) honor the same
  switch for *generation* but always *report* gaps (report-only when off).

### D3 — QA documentation gate (autospec-qa trio)

New top-level section **`## Documentation gate`** in the autospec-qa trio
(slotting with the existing proof/freshness gates): every QA run invokes
`/autospec-doc --full` (via `doc-orchestrator.mjs`), then `verify-examples.mjs`
across regenerated pages, then `check-doc-drift.sh --working-tree` — expecting
clean. Failures (generation error, failing example, residual drift, missing
audience page) are **QA findings** in the standard QA report format, with the
same severity discipline as the no-mock smoke rule. The `auto_regenerate`
switch is explicitly documented as NOT consulted here.

## Sequencing constraint

D2's gate conditional edits the **autospec-run trio** — the same surface as the
cost-efficiency epic's serialized chain (#938→#939→#941→#942). The D2 child
MUST depend on the cost epic's audit (#944). D1 (listen trio) and D3 (qa trio)
are independent surfaces and dep-free (config child first for D2/D3 consumers).

## Testing & validation

- bats: classifier positive/negative `fix` tables; config-key default +
  3-opt-in resolution matrix (incl. precedence with `docs: skip`); gate-block
  extraction `bash -n` (existing tests/phase4 pattern) asserting the
  conditional wraps ONLY the regenerate path.
- QA trio: named-content check in `validate.sh` for the `## Documentation
  gate` section across the qa trio; lock-step byte-identity for every trio
  edit (listen, run, qa — each authored in SKILL.md, mirrors regenerated).
- Regression guard: existing explore/discover + implement/build/ship routing
  tests stay green ('fix' must not cannibalize them).

## Risks

| Risk | Mitigation |
|---|---|
| 'fix' over-triggers on chatter | D4 gates + trivial suppressors + opt-out; negative bats table is the permanent guard |
| Docs silently rot with generation off | detection/labels stay on; QA always regenerates; Phase 5.5/sweep report gaps |
| Trio conflict with cost epic | D2 child depends on #944; trio edits serialized |
| Switch confusion (`docs: skip` vs `generate`) | documented precedence; resolution matrix test |

## Decomposition hint for /autospec-define

1. **Config key + resolution** (D2a): `auto_regenerate` default false +
   3-opt-in resolution helper + precedence tests. Dep-free.
2. **`fix` routing** (D1): classifier branch + suppressors + listen trio row +
   bats. Dep-free (listen trio quiet).
3. **Run-trio gate conditional** (D2b): wrap the #935 regenerate path; log
   line; Phase 5.5/sweep report-only mode. Depends on child 1 AND **#944**.
4. **QA documentation gate** (D3): qa trio section + validate named-content +
   QA-finding wiring. Depends on child 1.
5. Standard Phase 5.5 audit issue depending on all.

> Decomposer notes: three different trios are touched (listen/run/qa) — one
> trio per child, lock-step checkbox each; do NOT apply needs-autospec-template.
