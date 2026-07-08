# /autospec-autonomous Phase 2 — explore-backed discovery (design spec)

**Date:** 2026-06-26
**Builds on:** `docs/specs/2026-06-25-autospec-autonomous-design.md` (Phase 1 shipped
& proven; see its "Phasing & post-review corrections" — authoritative).
**Phase 1 status:** complete — conductor loop (`autospec_conductor_run()` in
`scripts/lib/autospec-loop.sh`), Tier 0 control + Tier 1 backlog→main, resilience
(lock/run-state/main-health/quarantine), `autospec-qa` pre-merge gate, cumulative
spend kill-switch. Phase 5.5 audit (#1380) confirmed the safety invariants hold.

## Phase-4 supersession note (2026-07-06)

`docs/specs/2026-07-06-autospec-autonomous-platform-design.md` supersedes Phase-2 dry-cycle park semantics. Discovery tiers are part of the default never-idle cascade; `AUTOSPEC_DISABLE_DISCOVERY_TIERS=1` is an emergency kill-switch, not the normal opt-in. Dry discovery results feed the value-ranked waterfall and eventually idle-rescan below `AUTOSPEC_VALUE_FLOOR`; they do not converge-stop. Blocking notify failures quarantine asynchronously and continue.

## Goal

Enable the conductor's **discovery tiers** so that when the backlog is dry it
continues the default never-idle cascade and generates its own high-value work — Tier 2 (local spec-vs-code discovery) and
Tier 3 (competitor reverse-engineering) — drains it onto an isolated **sandbox
branch** for batched operator review (never `main`), adds **security** to the
pre-merge gate, learns from an **outcome ledger**, and parks pre-emptively at a
**soft usage ceiling**. Phase 3 (operator persona/self-brainstorm) remains
deferred.

## Decisions carried in (locked Phase-1 brainstorm + peer review)

- **Self-generated work → sandbox**, backlog → `main` (merge-target-by-provenance).
- **Usage governor:** investigate live-usage observability per harness; use it if
  present, else token-tally fallback; spike first.
- **Competitor RE:** clean-room, behavior-only; `autospec-secaudit` IP backstop.
- **No nested perpetual loops:** the conductor drives discovery one cycle at a
  time via an `autospec-explore --once` interface (review-mandated).

## Prerequisites (verified 2026-06-26)

- `autospec-secaudit` — **already built** (repo source + global install). Phase 2
  only *wires* it into the gate; no build needed.
- `autospec-explore --once` single-cycle interface — **must be built** (explore
  today only has `--max-iterations`, which still files+drains and reports no
  dryness). This is Phase-2 child #1 and blocks Tier 2/3 enablement.

## Features

### F1 — `autospec-explore --once` single-cycle contract (PREREQ)

Add a `--once` mode to `autospec-explore` (and its orchestrator
`autospec-explore.sh` / `explore-research-cycle.sh`) that runs **exactly one**
research pass for a named source set and returns a machine-readable yield, WITHOUT
entering the perpetual loop and WITHOUT auto-draining:

```json
{"tier":"local|competitor","proposals_seen":N,"new_candidates":N,
 "filed":N,"dry":true|false,"reason":"..."}
```

`dry=true` when `new_candidates==0` after dedup against recently-filed issues. The
conductor calls this per cycle and counts consecutive `dry` results for tier
escalation. Trio change (SKILL.md + mirrors + goldens) + orchestrator script +
bats. This is the single seam that prevents the two-perpetual-loops hazard.

### F2 — Tier 2/3 enablement in the conductor

`scripts/autonomous-waterfall.sh`: replace the Phase-1 "not-yet-enabled" stubs for
Tiers 2-3 with real selection. After Tier 1 is dry, record the dry signal and rank
**Tier 2** (`explore --once` over local sources:
spec-vs-code, codebase-signals, source-analysis, dependency-health, prior-reports).
Tier 2 dry results feed **Tier 3** (`explore --once --research-sources
internet`) and then the Phase-4 quality/surface/RAG floors. A higher tier refilling (backlog issue appears) floats selection back
up next cycle. Wire into `autospec_conductor_run()`. `+` bats for the escalation
state machine.

### F3 — Self-generated work → sandbox routing

The conductor manages an explore sandbox branch (`autospec/explore/<date>-<slug>`
via `explore-sandbox.sh`, `.autospec/explore-mode.json`). When a discovery/
competitor cycle files issues, their implementer PRs target the **sandbox base**,
never `main` — reusing the existing Phase-4 sandbox contract in
`skills/autospec-run/prompts/phase4-implementer.md`. The conductor refuses
`gh pr merge` against `main` while discovery issues are in flight (identifier
`code_health:autonomous_main_merge_refused`). Backlog (Tier 1) still → `main`.

### F4 — secaudit in the pre-merge gate

Extend `scripts/autonomous-premerge-gate.sh`: after `autospec-qa`, also run
`autospec-secaudit`; same severity routing (high/severe/medium → fix-and-recheck
≤5; low/info non-blocking). Presence check still fails CLOSED if either skill is
absent. This completes the mandatory qa+secaudit barrier the Phase-1 spec
deferred. `+` bats.

### F5 — outcome-ledger wiring

Consult `autospec-explore-ledger` for dynamic source weights before ranking, and
feed per-source ship outcomes back after each cycle, so discovery biases toward
sources that historically merge clean and stops re-proposing rejected work.
Conductor calls the ledger's existing read/record interface; `+` bats.

### F6 — usage-governor spike + governor

- **Spike (child, Tier A):** determine per harness whether a live usage fraction
  is observable (Claude Code session/usage signals; Codex quota). Document the
  finding in the skill + README. Output gates F6b's mechanism.
- **Governor:** `scripts/autonomous-usage-governor.sh` — soft park at
  `AUTOSPEC_USAGE_SOFT_PCT` (default 90). If a live % is observable, use it; else
  fall back to the Phase-1 spend-ledger token tally at 90% of
  `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS`. On park: notify + arm
  `autospec-usage-limit.sh` resume. Layers on the existing hard-quota backstop.
  `+` bats.

### F7 — competitor config + clean-room internet researcher

`.autospec/competitors.yml` (or README/"purpose"-derived when absent) supplies
Tier-3 targets, constrained by the existing internet allowlist. The internet
researcher prompt enforces **behavior-level only — describe what a feature does,
never copy competitor source**; `autospec-secaudit`'s IP/license check (F4) is the
backstop. `+` bats for config parsing + the clean-room prompt guard.

### F8 — sandbox drift reporting + priority reweight

- Daily digest reports sandbox→`main` merge-base distance (commits/files behind)
  + conflict-risk estimate (no auto-rebase).
- Make the priority multiplier explicit: a `autospec:priority`/steer-aligned
  proposal's score = `base × AUTOSPEC_PRIORITY_BOOST` (default 1.5), bounded so it
  reorders without swamping `confidence × source_weight × 1/complexity`. `+` bats.

## Decomposition preview (≈8 children + 1 epic + Phase 5.5 audit)

1. **EPIC** — /autospec-autonomous Phase 2 (explore-backed discovery).
2. **F1** `autospec-explore --once` single-cycle contract (trio + orchestrator +
   goldens + bats). **Blocks F2/F3.** ctx:120k reasoning:deep.
3. **F2** Tier 2/3 enablement + escalation state machine in waterfall + loop wiring.
4. **F3** sandbox PR-base routing for self-generated work (+ main-merge refusal).
5. **F4** secaudit added to the pre-merge gate.
6. **F5** outcome-ledger wiring.
7. **F6** usage-governor spike (Tier A) **→ blocks** governor build (same child or
   a Depends-on pair).
8. **F7** competitors.yml + clean-room internet researcher guard.
9. **F8** sandbox drift reporting + priority reweight.
10. **Phase 5.5 audit + remediation** (never skip — `feedback_per_pr_lgtm_misses_integration`).

Each child ≤400 words, ≤3 logical units; trio prose + goldens atomic
(`feedback_decompose_trio_prose_goldens_atomic`); edit SKILL.md → `derive-trio.sh
--in-place` + `gen-skill-goldens.sh` (`feedback_skill_golden_derivation_workflow`).

## Tests

External boundaries (gh, explore, secaudit, notifier, usage API) mocked as
subprocesses. No gitignored fixtures; bats `[ -f ]` writes a real temp file first;
`set -eu` + if/then/fi; jq capture()/== not test().

- `tests/autonomous/test_waterfall.bats` (extend) — Tier 1 dry records observability
  then ranks Tier 2; Tier 2 dry feeds Tier 3/floor tiers; below value floor →
  idle-rescan; refill floats back up.
- `tests/explore/test_explore_once.bats` — `--once` runs one pass, emits the yield
  JSON, never enters the loop, never auto-drains.
- `tests/autonomous/test_sandbox_routing.bats` — discovery PRs target sandbox;
  main-merge refused while discovery in flight; backlog still → main.
- `tests/autonomous/test_premerge_gate.bats` (extend) — secaudit runs; high/med
  finding blocks; missing secaudit → halt.
- `tests/autonomous/test_usage_governor.bats` — observable-% parks at 90%; tally
  fallback parks at 90% of lifetime tokens.
- `scripts/validate.sh` — trio goldens regenerated; explore + autonomous contract
  gates extended.

## Self-review

- **Placeholders:** none.
- **Consistency:** backlog→main / self-generated→sandbox uniform; both new gates
  (secaudit, usage governor) fail safe (closed / park); `--once` removes the
  nested-loop hazard the review flagged.
- **Scope:** one multi-issue pipeline gated on F1 (the explore `--once` seam) and
  the F6 spike; both de-risk before dependents build.
- **Critical risks:** (a) nested perpetual loops → F1 `--once` contract;
  (b) unreviewed feature code on main → F3 sandbox routing + main-merge refusal;
  (c) IP risk in competitor RE → F7 clean-room + F4 secaudit; (d) runaway spend →
  F6 soft governor + Phase-1 hard kill-switch; (e) integration drift → Phase 5.5.
- **On merge:** update `docs/AUTONOMY-CHARTER.md` §5; flip Phase-2 items from
  "roadmap" to shipped in the Phase-1 spec's Phasing section.
