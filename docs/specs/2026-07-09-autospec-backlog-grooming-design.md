# Autospec Backlog Auto-Grooming — Design Spec

**Date:** 2026-07-09
**Repo:** github.com/berlinguyinca/autospec
**Author:** berlinguyinca
**Status:** Approved brainstorm; ready for implementation planning

## Goal

Make raw, human- or discovery-filed GitHub issues **automatically usable by the
autonomous loop** — i.e. autospec should groom its own backlog from "open but
un-implementable" to `auto-implement`-ready, without an operator hand-running
`/autospec-classify` and manually applying labels — while keeping a conservative
safety floor that holds genuinely-risky or ambiguous work for a human.

## Problem

`/autospec-run` only picks up issues that are `auto-implement`-labeled, ready
(deps met), and structurally sufficient. Today the transition from a raw issue to
that state is largely manual. Observed live on this repo (2026-07-09): with the
`auto-implement` queue drained to zero, **35 issues remained open, none usable by
the loop**:

| Bucket | Count | Why it's stuck |
|---|---|---|
| `needs-classify` **+** `needs-autospec-template` | 23 | Classified-but-untemplated **dead-end** (see below). |
| Unlabeled entirely | 8 | Never entered triage; several are real conductor bugs. |
| `epic` | 3 | Should be decomposed, not implemented directly. |
| `bug` only | 1 | Needs classify. |

The backlog is an assembly line whose **stages are labeled but whose transitions
don't all run automatically**.

## What already exists (reuse map)

This design is mostly **extension + wiring**, not greenfield. The following are
already built and MUST be reused, not reinvented (ROI discipline):

- **`scripts/autonomous-promote-open-issues.sh`** — the Tier 1.5
  `promote-open-issues` command. Already: selects open `needs-classify` issues →
  classifies via `classify-model-fit.sh` → adds `auto-implement` + `ctx:*` +
  `reasoning:*` + a `## Model fit` block → removes `needs-classify`. Has a
  report-only/apply split and a non-trivial-body gate.
- **`scripts/classify-model-fit.sh`** — deterministic model-fit classifier
  (stage 1). Reused as-is.
- **`docs/specs/2026-07-09-issue-intent-safety-gate-design.md`** — the fail-closed
  issue-intent safety gate (`scripts/lint-issue-safety.sh` + Tier-A semantic
  reviewer) producing `SAFETY_PASS` / `SAFETY_AMBIGUOUS` / `SAFETY_BLOCK`. Reused
  as the safety floor — grooming NEVER promotes an issue that is not
  `SAFETY_PASS`.
- **`/autospec-split`** — decomposes a spec/epic into linked child issues. Reused
  for epic handling.
- **`scripts/autonomous-waterfall.sh` Tier 1.5** — already emits
  `promote-open-issues` between Tier-1 drain and Tier-2 discovery. The grooming
  work lives **in this existing tier**, not a new one (capabilities are conductor
  tiers, not new conductors).

### The precise gaps this spec closes

1. **`needs-autospec-template` is a permanent dead-end.** The promoter *skips*
   `needs-autospec-template` (an excluded label), and nothing ever removes it. The
   23-issue bucket can never advance. **This is the single genuinely-new
   component: an LLM `groom-to-template` step that fills the required structural
   sections, clears `needs-autospec-template`, and hands the issue back to the
   existing promoter.**
2. **Env-lever gating.** The promoter mutates only when both `--apply` and
   `AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1` are set. Replace this with a declarative,
   self-governed `grooming:` block in `.autospec/autospec.yml`; the env var becomes
   a CI/test override only.
3. **Unlabeled issues are invisible.** The promoter only queries
   `needs-classify`. Extend candidate selection to also catch entirely-unlabeled
   open issues (apply `needs-classify` after a safety pass, then they flow the
   normal path).
4. **Epics are skipped, not decomposed.** Route `epic` issues to `/autospec-split`
   (when a spec/outline exists) instead of dead-ending them.
5. **No safety-gate integration.** Promotion must require `SAFETY_PASS` from the
   issue-intent safety gate.
6. **No discernment / no self-governance.** The promoter treats every
   non-excluded candidate identically and is all-or-nothing. Add a deterministic
   eligibility gate (legacy-drainable vs needs-templating vs hold) and a
   self-governing ratchet.

## Design

### Architecture

Grooming is an **extension of the existing Tier 1.5 `promote-open-issues`
command**, orchestrated by the same waterfall seam that #1632 made
readiness-aware. When Tier-1 ready-drain is empty but groomable issues exist,
Tier 1.5 grooms them into `auto-implement` **before** the loop descends to
discovery (Tier 2+) — "convert existing raw issues into ready work before
inventing new work."

```
Tier 1 (ready-drain)  ──empty──▶  Tier 1.5 GROOMING  ──still dry──▶  Tier 2+ (discovery)
                                        │
                    ┌───────────────────┴───────────────────┐
                    ▼                                        ▼
         deterministic front (B)                  LLM fallback (A)
```

### The grooming pipeline (per candidate issue)

Deterministic-first; LLM only where it buys something:

1. **Select** — `list-groomable.sh` (new, deterministic): open issues that lack
   `auto-implement`, carry no `hold:*` / `paused-by-user` / `locked-*` /
   `autospec:needs-human` / `wontfix` / `duplicate` label, and are either
   `needs-classify`, `needs-autospec-template`, or **unlabeled**. Oldest-first,
   capped at `grooming.budget.max_issues_per_cycle`. Dedupes against **closed
   no-op findings** by title/body hash (directly addresses #1470).
2. **Safety gate (floor)** — run the issue-intent safety gate. `SAFETY_AMBIGUOUS`
   / `SAFETY_BLOCK` → apply `security:quarantined`, remove any queue labels, post
   a comment, and **stop** for this issue. Only `SAFETY_PASS` proceeds.
3. **Classify** — reuse `classify-model-fit.sh` → add `ctx:*` / `reasoning:*`,
   drop `needs-classify`. (Unlabeled issues get `needs-classify` applied first,
   then classified in the same pass.)
4. **Eligibility gate** — `promote-eligibility.sh` (new, deterministic,
   **fail-closed**): is this issue legacy-drainable **as-is**? Signals: actionable
   body (length + a clear `fix:`/`feat:` intent or a repro/expected-behavior),
   single-or-few-file scope estimate, not multi-subsystem, not epic.
   - **Eligible → promote now** (`auto-implement`, drop `needs-autospec-template`
     if present), **no LLM** — because a plain bug drains fine on the legacy Phase-4
     path without a `### Primary smoke test` section.
   - **Not eligible but groomable → step 5.**
   - **Uncertain → hold** (fail-closed).
5. **LLM groom-to-template (the one new LLM step)** — for groomable-but-not-eligible
   issues: a Tier-B subagent fills the required template sections (Files-to-read /
   Implementation-outline / Tests-required / **Primary smoke test**) from the
   issue body + a pattern survey, writes them into the issue body, then a
   **deterministic template linter validates** the result with **5× adaptive
   retry** (findings fed back as directives — same pattern as every autospec LLM
   validator). Pass → drop `needs-autospec-template`, promote. Still failing after
   the cap → hold.
6. **Epics** → if the issue carries a spec/outline, route to `/autospec-split`
   (auto-decompose into child issues, which then re-enter grooming). Otherwise
   `hold:epic` + comment "run /autospec-define to produce a spec".
7. **Floor (held for human)** — ambiguous scope (two valid readings),
   high-blast-radius (touches security / migrations / the skill trio /
   `validate.sh` / many files), **unresolvable dependency** (a `Depends on #N`
   citing a nonexistent/closed-won't-fix issue — catches #1627's bogus `#3025`),
   or groom-failed → `hold:needs-human` + a comment stating exactly what's missing.

### Config & self-governance

One declarative block, no env-lever farm:

```yaml
# .autospec/autospec.yml
grooming:
  policy: auto            # auto | on | off   (default: auto)
  budget:
    max_issues_per_cycle: 5
    groom_attempts_per_issue: 2   # LLM template attempts before hold
```

`policy: auto` self-governs, reusing the advisor ratchet shape
(`advisor-govern.sh` generalized or cloned):

- **Seed conservative:** auto-promote only high-confidence *eligible* (step-4)
  issues; the LLM-template-auto-promote (step 5) is gated off at seed.
- **Tick each grooming cycle:** measure the **groomed-issue clean-merge rate** —
  auto-promoted issues whose implementation PR merged with no revert, no
  reopened-rework, no `escalate:human` — against a baseline of human-labeled
  issues' clean-merge rate, over a min-sample floor.
- Quality ≥ baseline **and** within budget → **widen** (enable step-5
  auto-promote; loosen the eligibility threshold one notch).
- Regression (groomed issues merge worse than the baseline) → **tighten** back
  toward the seed. Never below the seed.

`AUTOSPEC_GROOMING_*` env vars exist only as CI/test overrides.

### Label lifecycle (state machine)

```
(unlabeled) ──safety PASS──▶ needs-classify ──classify──▶ (classified)
                                                              │
                        ┌──────────── eligible ──────────────┤
                        ▼                                     ▼ not eligible
                  auto-implement                     needs-autospec-template
                        ▲                                     │ groom+lint OK
                        └──────────── (drop template) ────────┘
   any step: SAFETY_AMBIGUOUS/BLOCK ▶ security:quarantined
   ambiguous / high-blast / unresolvable-dep / groom-fail ▶ hold:needs-human
   epic ▶ /autospec-split (children re-enter) | hold:epic
```

### Guardrails

- Never touch `hold:*`, `no-auto`, `security:quarantined`, or human-`paused` issues.
- Per-issue groom-attempt cap (`groom_attempts_per_issue`); a held issue is not
  re-groomed until a human clears the hold.
- Every promote/hold/quarantine posts an **audit comment** with the decision +
  reasons.
- Eligibility and safety are **fail-closed**: uncertainty → hold/quarantine,
  never promote.
- Grooming mutates GitHub only when `policy` resolves to `auto`/`on`; `off` and
  the report-only path mutate nothing (dry JSON only), preserving the existing
  double-gate safety for tests.

## Effect on the current 35-issue backlog

- **23 `needs-classify`+`needs-autospec-template`:** safety-passed, classified;
  the legacy-drainable subset auto-promotes immediately; the rest are
  LLM-templated → promoted, or held with a reason.
- **8 unlabeled:** enter triage (safety → `needs-classify` → classify → same path).
- **3 epics:** → `/autospec-split` (if spec'd) or `hold:epic`.
- **#1664:** classify + promote.
- **#1627:** `hold:needs-human` (unresolvable `#3025` dependency).

## Testing

- **TDD bats per deterministic script:** `list-groomable.sh` (selection/dedup/
  exclusion fixtures), `promote-eligibility.sh` (eligible / not-eligible /
  uncertain-fail-closed fixtures), the govern tick (promote/hold/retract on seeded
  telemetry). No live LLM and no live GitHub writes in tests — stub `gh` and the
  groom subagent; assert on emitted JSON + intended label mutations.
- **Groom-to-template step:** tested via its deterministic template linter +
  retry harness against fixture issue bodies (LLM stubbed); assert the linter
  gates a bad body and the retry feeds findings back.
- **Safety-gate + eligibility fail-closed:** explicit negative cases (a
  hostile-intent fixture is quarantined; an ambiguous body is held).
- **Live acceptance (dogfood):** run the grooming tier against this repo's own
  35-issue backlog and assert the bucket outcomes above.

## Non-goals

- Not a new conductor or a new operator-facing skill — it's an extension of the
  existing Tier 1.5 command (an internal helper set), self-governed via config.
- Does not replace `/autospec-classify`, the safety gate, `autospec-split`, or the
  discovery tiers — it composes them.
- Does not implement issues — promotion hands off to the normal Phase-4 loop.
- No new env-var levers beyond CI/test overrides.

## Open questions / dependencies

- **Depends on** the issue-intent-safety-gate (`lint-issue-safety.sh`) being
  implemented, or the grooming safety step degrading to "hold on any uncertainty"
  until it lands. Confirm the gate's status before planning.
- The template linter that sets `needs-autospec-template` today — locate it and
  reuse it verbatim as the groom-to-template validator (single source of truth for
  "is this templated?").
- Whether the self-gov ratchet clones `advisor-govern.sh` or generalizes it into a
  shared `govern.sh` primitive (decide at plan time; prefer generalize if the
  shapes are identical).
