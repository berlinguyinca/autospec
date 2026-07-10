# AutoSpec Growth — fold into the autonomous conductor (Plan 5 of 5, revised)

**Status:** Design
**Date:** 2026-07-10
**Author:** berlinguyinca
**Depends on:** Plan 1 foundation, Plan 2 pipeline, Plan 3 grow-define, Plan 4 grow-run
**Supersedes:** the standalone `/autospec-grow` conductor design (dropped — see Rationale)
**Completes:** the 5-plan AutoSpec Growth capability

## Summary

The perpetual growth loop is **not** a new skill. Instead of building a parallel
`/autospec-grow` conductor, growth folds into the existing `autospec-autonomous`
conductor as **capability-gated tiers**, following the platform's shipped F4–F8
extension model. `grow-define` and `grow-run` stay as the reusable halves —
exactly as `define`/`run` relate to `autonomous` today — and the conductor
invokes them under the hood. This plan also fixes a **live bug** the fold-in
exposes.

## Rationale (why fold in, not build a conductor)

Four independent code-reads converged:

1. **The project's own doctrine.** `autospec-autonomous`'s SKILL.md states: *"This
   skill is a conductor, not a new engine. It reuses without reimplementing:
   `autospec-run`…"*; `SKILLS.md`: *"Use the narrowest skill that matches the
   job."* A second conductor is the "new engine" the project tells itself not to
   build. The surface is already ~34 skills.
2. **A shipped extension model exists.** F4–F8 (architecture-fitness, mutation,
   security, UX/a11y, docs/RAG) all landed as capability-gated tiers: one
   `_action` dispatch, one `AUTOSPEC_*_CMD` env seam, one dry-cycle/park
   mechanism, gated on a `.autospec/*.yml` opt-in, scored through the shared
   `autonomous-prioritize.sh` WSJF ranker. Growth is the same shape.
3. **A separate conductor duplicates** the Tier-0 control channel, premerge/
   main-health gates, resilience/locking, digest/notify, and quota machinery —
   and two conductors compete for one operator's quota, defeating the single
   ranked waterfall.
4. **Live bug.** `growth:artifact` issues carry `auto-implement`
   (`grow-define-file-issues.sh:25`), so `autospec-autonomous` Tier 1 **already
   drains them today** — silently bypassing the content-quality gate stranded in
   `grow-run` R1. This must be fixed regardless of the conductor question.

### Design decisions (locked with the operator)

- **Fold growth into `autospec-autonomous` as tiers; no new `/autospec-grow`
  skill.**
- **Shared WSJF budget** — growth work competes against code-health/backlog work
  in the one waterfall under one quota (no separate growth budget seam).
- **One combined plan** — the route-fix and the tier fold-in ship as one
  reviewed branch.

## Part A — Fix the content-quality gate bypass

The content-quality gate currently lives only in `grow-run` R1, which assumes it
is the only path a `growth:artifact` issue takes to `/autospec-run`. It isn't:
the autonomous Tier-1 drain reaches those issues directly. Fix by moving the gate
decision into `/autospec-run`'s existing Phase-4 label router so it fires for
**every** path.

1. **Generalize `skills/autospec-run/scripts/fab-route.sh`** from a `fab|default`
   router to a `fab|growth|default` router: an issue whose labels contain
   `growth:artifact` routes to `growth`. Whole-label match (no substring), bash
   3.2-safe, deterministic. Precedence when an issue carries both a fab and a
   growth label: `fab` > `growth` > `default` (documented; disjoint in practice).
   Extend the bats suite with the growth cases.
2. **`/autospec-run` Phase-4 prose** — add a `GATE=growth` branch: run the
   content-quality gate (`growth-content-quality-precheck.sh` deterministic
   pre-checks → Tier-A reviewer → 5-attempt adaptive-retry) as an **additive**
   gate before the standard reviewer + `growth-ethics` + `autospec-secaudit`
   gates that already run for every issue. This closes the gap for both the bare
   Tier-1 drain and `grow-run` R1 with one implementation. Re-derive the
   `autospec-run` trio + regenerate goldens (bare-name `gen-skill-goldens.sh`).
3. **Simplify `grow-run` R1** — since `/autospec-run` now gates `growth:artifact`
   issues itself, `grow-run` R1 no longer layers the gate separately; it invokes
   `/autospec-run` (which now gates) and drops the duplicated gate prose. This
   removes a maintenance seam and makes the gate single-sourced. Re-derive the
   `grow-run` trio + goldens.

## Part B — Fold growth into the conductor as capability-gated tiers

Capability detection: all growth tiers are **inert unless `.autospec/growth.yml`
exists and validates** (`validate-growth-config.sh`), mirroring how F8 web/RAG
tiers activate only when the surface is detected. Non-growth repos see **zero**
behavior change.

Each tier follows the existing recipe: a waterfall `action` +
`elif [ "$_action" = "…" ]` branch in `scripts/lib/autospec-loop.sh` with an
`AUTOSPEC_<CAP>_CMD` seam (env override → real skill invocation → graceful
no-op), a `_tierN_dry_cycles` counter, and `_work_done`. **New tiers are appended
after Tier 4** to avoid renumbering the tier-number contracts existing tests and
digests assert.

### Tier G1 — Growth discovery (→ `grow-define`)

- **Waterfall:** `scripts/autonomous-waterfall.sh` emits
  `{"tier":<next>,"action":"run-growth-define",…}` when growth is enabled and the
  growth backlog is below `grow.backlog_floor` (config, default 3), gated by its
  own dry-cycle counter like Tier 3.
- **Loop:** `AUTOSPEC_GROWTH_DEFINE_CMD` seam; fallback invokes
  `/autospec-grow-define` one cycle. It files `growth:artifact` /
  `growth:outbound` issues; artifacts then flow into ordinary Tier 1 (already
  happens), gated by Part A.
- Candidates are scored through the shared `autonomous-prioritize.sh` ranker so
  growth work competes fairly against code-health/backlog work under one quota.

### Tier G2 — Outbound approval service (→ `grow-run` R2/R3)

- **Waterfall:** emits `service-growth-outbound` when there are open
  `growth:outbound` issues to draft OR `growth/needs-approval` control issues
  with a human decision label (`growth/approved|edited|rejected`) to service.
  This is a cheap, Tier-0-style poll evaluated each cycle boundary.
- **Loop:** `AUTOSPEC_GROWTH_OUTBOUND_CMD` seam; fallback invokes `grow-run`'s
  R2/R3 (draft → ethics/cadence/relevance gate → queue; and handle approvals).
  **Never auto-posts** (Plan 4 invariant preserved verbatim) — approval produces
  a package for the human.

### Tier G3 — Measure & attribute (→ `grow-run` R4)

- **Cadence-gated**, not per-cycle: runs when `grow.measure_interval` (config,
  default 14 days) has elapsed since the last measure line in the growth ledger
  — modeled on the conductor's existing once-per-UTC-day digest, not a
  priority-competing tier.
- **Loop:** `AUTOSPEC_GROWTH_MEASURE_CMD` seam; fallback invokes `grow-run` R4
  (adapters → normalize → attribute → re-weight lenses → learnings memo).

### Conductor surface updates

- `scripts/autonomous-waterfall.sh` — the three new actions + their dry-cycle /
  cadence gates, all behind the growth-enabled check.
- `scripts/lib/autospec-loop.sh` — three new `elif` dispatch branches with their
  seams and counters, appended to the existing tier dispatch.
- `skills/autospec-autonomous/SKILL.md` — extend the description/tier
  documentation to mention the growth tiers (capability-gated). Re-derive the
  trio + regenerate goldens.
- No new spend/park/resilience/control-channel code — reused as-is.

## Reuse map

**Reused (no fork):** the entire conductor engine (loop, waterfall cascade,
Tier-0 control channel, spend-ledger, usage-governor, resilience, digest,
`autospec-usage-limit` park/resume), `autonomous-prioritize.sh` (WSJF),
`/autospec-grow-define`, `/autospec-grow-run`, `/autospec-run`,
`growth-ledger.sh`, `validate-growth-config.sh`, `growth-content-quality-precheck.sh`.

**New:** the `growth` route in `fab-route.sh`; the `GATE=growth` branch prose in
`/autospec-run`; three waterfall actions + three loop dispatch branches; the
growth-enabled capability gate. No new skill, no new conductor, no new budget
machinery.

## Error handling (fail-closed / never-idle)

- No/invalid `growth.yml` → growth tiers inert; the conductor behaves exactly as
  today.
- A growth tier's dispatched cycle fails → logged; the conductor records it and
  continues at the next cycle boundary (convergence-stop forbidden; only spend/
  usage/operator-control park).
- The content-quality gate is fail-closed (Plan 4): an unparseable draft or a
  gate error blocks merge, never ships unreviewed growth content.
- Never auto-posts (Plan 4 invariant), enforced in Tier G2.

## Testing

- **Route fix (bats):** `fab-route.sh` returns `growth` for `growth:artifact`,
  `fab` for fab labels (unchanged), `default` otherwise; precedence when both;
  whole-label match (`growth:artifactx` → default).
- **Waterfall growth tiers (bats):** with growth enabled, backlog below floor →
  `run-growth-define`; open outbound/approval work → `service-growth-outbound`;
  measure interval elapsed → `run-growth-measure`; growth disabled (no
  `growth.yml`) → **none** of the growth actions ever emit (regression: existing
  tier selection byte-identical). Build ledger state via real
  `growth-ledger.sh --append`.
- **Loop dispatch (bats):** each growth `_action` routes to its seam; a growth
  tier failure doesn't stop the loop; growth-disabled repos exercise the
  unchanged path.
- **Trio + goldens:** `autospec-run`, `autospec-grow-run`, and `autospec-autonomous`
  SKILL.md edits each re-derived (`derive-trio.sh skills/<skill> --in-place`) and
  regenerated (`gen-skill-goldens.sh <skill>` — bare name).
- **`validate.sh`** green (full suite); **root `install.sh --hook-mode`** green
  (no new skill pair; existing pairs unaffected).
- **Regression:** a repo without `.autospec/growth.yml` produces byte-identical
  waterfall decisions and conductor behavior to pre-change `main` — the fold-in
  is invisible unless opted in.

## Non-goals

- No new `/autospec-grow` skill, no standalone growth conductor, no separate
  growth budget.
- No changes to the growth research/pipeline/adapters (Plans 1–4).
- No auto-posting; publishing stays package-only.

## Open questions

- `grow.backlog_floor` (3) and `grow.measure_interval` (14 days) — tune from real
  cycles.
- Whether Tier G2 outbound polling belongs closer to Tier 0 (always-poll) than a
  ranked tier — default: a low-cost ranked action; revisit if approval latency
  matters.
- Whether growth candidates need a WSJF field mapping distinct from code issues
  (ROI/severity → WSJF) — start by reusing the existing ranker inputs; refine if
  growth work is systematically mis-ranked against code work.
