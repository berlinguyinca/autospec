# /autospec-autonomous — Never-Idle, Never-Ask Autonomous Software Architecture Platform (design spec)

**Date:** 2026-07-06
**Builds on:** `docs/specs/2026-06-25-autospec-autonomous-design.md` (Phase 1), `docs/specs/2026-06-26-autospec-autonomous-phase2-design.md` (Phase 2), `docs/specs/2026-06-26-autospec-autonomous-phase3-design.md` (Phase 3), `skills/autospec-autonomous/SKILL.md`.
**Supersedes:** the dry-cycle **park-and-notify** floor, the **blocking notify-operator** failure model, and the startup **`AskUserQuestion`** gate from the above. **Extends** everything else (waterfall tiers, trio/goldens authoring, model-tier routing, sandbox-by-provenance, qa+secaudit premerge gate).
**Phase status:** Phase 4 — platform generalization. Phases 1–3 shipped and Phase-5.5-audited.
**Review status:** draft for decomposition. Tracked by epic #1531 (its workstreams #1532–#1552 are this spec's decomposition).

## Goal

Turn `/autospec-autonomous` from a backlog drainer that **parks when the `auto-implement`
queue is empty** into a **complete autonomous software-architecture platform** that runs
unattended indefinitely and, at every cycle, does the single highest-value thing available —
draining backlog, promoting existing issues, discovering new work, or continuously improving
code, architecture, tests, performance, security, UX, accessibility, documentation, and the
RAG knowledge base built from that documentation. It **never stops for lack of work** and it
**never asks the operator** for a decision it can evaluate itself. Safety is enforced by
asynchronous fences, not by blocking prompts.

This behavior must be **native to autospec and environment-agnostic**: no target-repo
specifics are hardcoded; every capability, fence, and threshold is config-driven and
capability-detected per repo.

## Non-goals

- Not a rewrite of the conductor. This **extends** `autospec_conductor_run()` in
  `scripts/lib/autospec-loop.sh` and the existing `autonomous-*.sh` scripts; it does not
  replace them.
- Not "always busy." Manufacturing low-value churn to avoid idling is an explicit
  anti-goal (see F1/F2). Activity is not the success metric; value shipped is.
- Not removing resource limits. The cumulative spend kill-switch, usage governor, and
  operator `autospec:pause` still park the loop. Never-idle governs **work availability**,
  not **resource exhaustion** (see Reconciliation §R5).
- Not target-repo-specific. Trading/finance/web specifics are examples only; the shipped
  behavior is generic and gated by per-repo config + capability detection.

## Locked decisions (operator, 2026-07-06)

1. **Never converge-stop.** A dry `auto-implement` queue is not a terminal state. The loop
   descends the waterfall to the next tier with value-positive work and, when everything is
   below the value floor, **idles on a re-scan heartbeat** — it never parks *for lack of work*.
2. **Never ask.** The conductor must not emit `AskUserQuestion` or block on operator input
   for any planning, decomposition, prioritization, or proceed/approve decision. "Ask the
   user" is a failure mode. Operator steering is out-of-band via Tier-0 control labels.
3. **Fences are async quarantine, not prompts.** A change exceeding the blast-radius policy
   is quarantined into a `autospec:needs-human` review queue and the loop **continues to the
   next work item** — never a modal "may I proceed?".
4. **Every improvement carries a measurable before/after signal.** An LLM "this looks
   better" is a proposal, never a self-approved merge. No signal → no merge.
5. **Value-gated selection replaces dry-cycle escalation as the primary selector.** Tiers
   still exist, but within/across them the conductor picks the max-value item; a value floor
   decides idle vs. act.

## Reconciliation with prior specs

The prior Phase-1–3 docs and `SKILL.md` contain semantics this spec overrides. Each is
resolved explicitly so the trio (SKILL.md + codex/opencode mirrors) and goldens stay coherent.

- **R1 — Park-on-dry → value-floor idle.** SKILL.md's Phase-1 contract "on `dry_cycle >= 2`
  the conductor parks and notifies the operator" is replaced: on dryness the conductor
  descends to the always-available quality/standards floor (F4–F6); only when *all* tiers
  yield nothing above `AUTOSPEC_VALUE_FLOOR` does it enter re-scan idle (F1). Discovery is no
  longer gated behind `AUTOSPEC_ENABLE_DISCOVERY_TIERS` opt-in by default — the platform
  build enables the cascade; the flag becomes a kill-switch, not an opt-in.
- **R2 — Blocking notify → async quarantine.** Every failure path that currently
  "halts + notifies + waits" (`autospec:needs-human`, gate-missing, main-red, unfixable
  secaudit) is restated as **quarantine-and-continue**: file/label, record `code_health:*`,
  and proceed to the next item. Only resource-park (R5) and `autospec:stop`/`pause` halt.
- **R3 — Startup question removed.** The Phase-1/Phase-3 startup `AskUserQuestion` (when no
  `--priorities` given) is removed; the conductor infers priorities from
  `autonomous-priorities.md`, the operator persona, and control labels, and proceeds.
- **R4 — Kill-switch is authoritative.** Where Phase-1 prose says "runs forever, no cost
  kill-switch" but the AUTHORITATIVE section mandates a cumulative spend kill-switch, this
  spec adopts the kill-switch reading: never-idle ≠ never-park-on-resource-limit.
- **R5 — Two orthogonal axes.** Distinguish **convergence-stop** (forbidden — stopping
  because no work is left) from **resource-park** (allowed — pausing because spend/usage/
  operator says so). Never-idle forbids the former; the governor/ledger own the latter.

## Core model

### Two axes
- **Work-availability axis** (this spec): the loop always has a next action while any
  value-positive work exists; below the floor it idles-and-rescans. It never converge-stops.
- **Resource axis** (unchanged): `autonomous-spend-ledger.sh` (hard lifetime cap) and
  `autonomous-usage-governor.sh` (soft `AUTOSPEC_USAGE_SOFT_PCT`, default 90%) park the loop;
  `_conductor_arm_resume()` re-arms it. `autospec:pause`/`stop` park/exit at a cycle boundary.

### The value-gated waterfall (one cycle)
`autonomous-control-channel.sh` (Tier-0 preempt) → build the candidate set from all enabled
tiers → `autonomous-prioritize.sh` ranks every candidate by one WSJF-style score (F2) →
select the max; if `max_score < AUTOSPEC_VALUE_FLOOR` → **idle-rescan** (F1) → else
`autonomous-premerge-gate.sh` → blast-radius classify (F7); fenced → quarantine + next →
drain via `/autospec-run` (base by provenance) → post-merge health + auto-rollback (F7) →
`autonomous-spend-ledger.sh` → `autonomous-resilience.sh` → digest.

Candidate tiers (superset of the existing Tier 0–4):
- **Tier 0** Control channel · **Tier 1** `auto-implement` backlog · **Tier 1.5** promote
  existing issues (decompose epics, unblock deps, classify) · **Tier 2** local discovery
  (`/autospec-explore --once`) · **Tier 3** competitor RE · **Quality floor** (F4) ·
  **Surface & knowledge** (F5–F6) · **Tier 4** polish lenses.

The "floor" tiers (F4–F6) are **always non-dry** for a non-trivial repo (there is always a
coverage gap, a debt hotspot, a doc drift, a11y finding, or a RAG eval regression above or
below the value floor), which is what guarantees never-idle without manufacturing churn:
the floor produces *real, measured* work or nothing.

## Features

### F1 — Never-idle invariant + value-floor idle
- Replace the dry-cycle park in `autospec_conductor_run()` with: descend tiers; if the
  best candidate across all enabled tiers scores below `AUTOSPEC_VALUE_FLOOR`, enter
  **idle-rescan** — write resume context, `notify.sh` (async, informational), and re-arm a
  cheap heartbeat via `_conductor_arm_resume()` at `AUTOSPEC_RESCAN_INTERVAL` (default 30m).
  Idle is a first-class state, distinct from resource-park; the digest names which.
- Convergence-stop is impossible: the floor tiers are always evaluated. A dry cycle counter
  survives only as observability (`AUTOSPEC_AUTO_DRY_CYCLES` no longer triggers park).

### F2 — Value-gated prioritization engine (WSJF)
- New `autonomous-prioritize.sh` scores every candidate:
  `score = (Severity × Value × Confidence × Reversibility) / (Effort × BlastRadius)`.
  - Severity/Value: CVE CVSS, failing gate, prod-signal, SLA/perf breach outrank cosmetics.
  - Confidence: verified signals (failing bench, active advisory, eval regression) beat
    speculative refactors — folds in the existing `confidence × source_weight` from the
    explore ledger.
  - Reversibility & BlastRadius are **divisors** — cheap-to-revert, single-module work floats
    up; fenced/high-radius work sinks or routes to quarantine (F7).
- **Idle floor:** below `AUTOSPEC_VALUE_FLOOR`, don't act — idle-rescan (F1).
- **Anti-thrash:** decay recently-touched paths to prevent A→B→A ping-pong; per-PR diff
  cap; single-concern enforcement. Publish the ranked queue (incl. considered-and-skipped)
  in `.autospec/autonomous-digest.md`.

### F3 — No-ask / async quarantine model
- Remove `AskUserQuestion` from the conductor path (R3). All operator interaction is
  out-of-band: read `autospec:priority|steer|pause|stop|needs-human` at cycle boundaries;
  infer intent from `autonomous-priorities.md` + `operator-persona.md`.
- Restate every blocking-notify failure path as quarantine-and-continue (R2), preserving the
  existing `code_health:*` identifiers as async signals, not halts.

### F4 — Always-available code-quality floor
First-class recurring tiers, each emitting measured candidates into the F2 ranker. Enabled
per repo via `.autospec/autonomous.yml`; each requires a **before/after signal** (F8/locked #4).
- **Architecture fitness functions** (Ford/Parsons): declarative registry enforced as CI
  gates — layering/no-cycles, invariant assertions (generalize `assert_not_impl_any!`
  patterns), latency budgets, coupling thresholds, async-safety; a breach files a candidate.
- **Coverage & mutation testing:** prefer **mutation score** (`cargo-mutants`/equivalent)
  over line coverage; a surviving mutant → a candidate test that kills it (fails-then-passes).
- **Debt / dead-code / dependency-CVE:** churn×complexity hotspots, dead code
  (test-only-referenced = dead), advisories via audit tooling.
- **Performance:** continuous benches with per-commit baselines; a significant regression
  files a candidate; optimizations carry a reproducible before/after delta.
- **Security:** SAST, secret scan, unsafe-surface audit as proactive discovery, not only a
  per-PR gate (`autospec-secaudit` stays the gate).

### F5 — Surface & knowledge tiers (extend the Tier-4 lenses into signal-gated tiers)
Gated by **capability detection** (only run for repos that have the surface):
- **UX/UI optimization** (web surface): deterministic backbone — Lighthouse CI, Core Web
  Vitals (LCP≤2.5s, INP≤200ms, CLS≤0.1), design-token lint, visual-regression, HEART/funnel
  signals when instrumented; LLM heuristic reviewer whose suggestions must be validated by a
  hard signal before shipping.
- **Accessibility & web-standards** (web surface): WCAG 2.2 AA via axe-core/pa11y/Lighthouse/
  IBM; severity model `legal_exposure × user_blocking × traffic × occurrences`; auto-remediate
  the machine-verifiable class, quarantine the judgment class; adjacent standards (structured
  data, security headers, privacy/consent UX, i18n). Both themes validated where themed.
- **Documentation freshness:** docs-as-tests (examples compile/run), drift detection
  (public API/config changed, docs stale), per-audience single-source, `llms.txt`/`llms-full.txt`.

### F6 — RAG documentation database + eval-gated tuning (net-new)
A retrieval index over the repo's docs (consuming F5's clean corpus), continuously tuned:
- **Levers as config knobs:** structure-aware + late/contextual chunking; embedding model
  (index versioned by `(model_id, chunk_config_hash)`, no mixed spaces); hybrid dense+BM25
  with RRF + reranking + metadata filters; query-transform router (rewrite/HyDE/multi-query);
  incremental freshness (content-hash invalidation, versioning).
- **Eval harness as the gate:** in-repo golden set `{question, ideal_answer,
  relevant_chunk_ids}`; retrieval metrics (nDCG/MRR/recall) every commit + RAGAS
  faithfulness/relevancy nightly. A knob change self-promotes only on target-metric gain with
  **no faithfulness regression** (floor e.g. 0.90); breaches quarantine (F7). Citation
  verification: every claim maps to a cited chunk whose span supports it.

### F7 — Autonomy guardrails hardening
- **Immutable verifier:** test files and the eval harness are read-only to the implementer
  lane; a diff-guard rejects implementer edits to assertions/harness; mutation-score drop
  blocks merge (catches assertion-gutting without file edits).
- **Blast-radius classifier + async quarantine:** classify each diff from touched paths
  against a **config-driven fenced-surface registry** (`.autospec/fenced-surfaces.yml`) —
  reversible+low-radius auto-merges; fenced/high-radius → `autospec:needs-human` queue +
  continue (never a prompt).
- **Post-merge auto-rollback + provenance:** a red post-merge health signal auto-reverts and
  files a follow-up; every autonomous merge records provenance + gate evidence + a rollback
  handle.
- **Separation of powers:** author ≠ verifier ≠ approver, enforced and audited; verifier is
  adversarial (refute-by-default).

### F8 — Environment-agnostic configuration
- `.autospec/autonomous.yml` (per repo): which quality/surface tiers are enabled, the value
  floor, re-scan interval, fenced-surface registry path, capability hints.
- **Capability detection:** web tiers (F5 UX/a11y) only activate when a web surface is
  detected; RAG (F6) only when docs exist; language-specific tools resolved per stack.
- No target-repo specifics in scripts or SKILL.md — all examples in prose are labeled as
  examples; defaults are safe no-ops when a capability is absent.

## Decomposition preview

1. **EPIC** — #1531 Never-Idle Autonomous Software Architecture Platform (this spec's tracker).
2. F1 never-idle + value-floor idle — extends `autospec_conductor_run()` (supersedes park-on-dry). ↔ #1542
3. F2 `autonomous-prioritize.sh` value-gated engine + anti-thrash. ↔ #1542
4. F3 no-ask + async-quarantine restatement across conductor + failure paths. ↔ #1543/#1545
5. Tier-2 enabler — single-cycle `/autospec-explore --once` wiring. ↔ #1532
6. F4 architecture fitness functions. ↔ #1533
7. F4 coverage & mutation testing. ↔ #1534
8. F4 debt / dead-code / dependency-CVE. ↔ #1535
9. F4 performance & benchmark-regression. ↔ #1536
10. F4 security / secret / compliance scanning. ↔ #1537
11. F5 UX/UI optimization. ↔ #1538
12. F5 accessibility & web-standards. ↔ #1539
13. F5 documentation freshness. ↔ #1540
14. F6 RAG database + eval-gated tuning (subtree). ↔ #1541 (#1548–#1552)
15. F7 guardrails: immutable verifier / blast-radius quarantine / rollback / separation. ↔ #1543 (#1544–#1547)
16. F8 environment-agnostic config + capability detection.
17. Trio + goldens: `derive-trio.sh --in-place` regenerates SKILL.md mirrors; `gen-skill-goldens.sh`; reconcile R1–R5 in SKILL.md + phase docs.
18. **Phase 5.5 audit** — verify no target-repo specifics leaked; every tier emits a measured signal; no blocking prompt remains; `autospec validate` green.

## Tests

- `tests/autospec/test_conductor_never_idle.bats` — dry backlog descends to floor, never parks for lack of work; below floor → idle-rescan, not park.
- `tests/autospec/test_prioritize.bats` — WSJF ordering; value-floor gate; anti-thrash decay prevents A→B→A.
- `tests/autospec/test_no_ask.bats` — conductor path emits no `AskUserQuestion`; failure paths quarantine-and-continue.
- `tests/autospec/test_blast_radius_quarantine.bats` — fenced-surface diff quarantined, loop continues; low-radius auto-eligible.
- `tests/autospec/test_immutable_verifier.bats` — implementer edit to a test/harness rejected; mutation-score drop blocks.
- `tests/autospec/test_auto_rollback.bats` — red post-merge signal reverts + files follow-up.
- `tests/autospec/test_rag_eval_gate.bats` — config promoted only on metric gain + no faithfulness regression; stale/uncited answers flagged.
- `tests/autospec/test_capability_detection.bats` — web/RAG tiers no-op when the surface/docs are absent.
- Trio golden parity: `tests/fixtures/skill-goldens/autospec-autonomous.*.sha256`.

## Self-review

- **Placeholders:** none.
- **Consistency:** reuses canonical names (`autospec_conductor_run`, `autonomous-waterfall.sh`,
  `autonomous-control-channel.sh`, `autonomous-premerge-gate.sh`, `autonomous-spend-ledger.sh`,
  `autonomous-usage-governor.sh`, `autonomous-resilience.sh`, `_conductor_arm_resume`,
  `derive-trio.sh --in-place`, `gen-skill-goldens.sh`, control labels, `code_health:*`). New
  artifacts: `autonomous-prioritize.sh`, `.autospec/autonomous.yml`,
  `.autospec/fenced-surfaces.yml`, env `AUTOSPEC_VALUE_FLOOR`, `AUTOSPEC_RESCAN_INTERVAL`.
  R1–R5 reconcile every superseded semantic against SKILL.md + the phase docs.
- **Scope:** extends the conductor and adds tier engines; does not rewrite the loop, gates,
  model-tier routing, or sandbox-by-provenance.
- **Critical risks:** (a) never-idle degenerating into churn — mitigated by the value floor +
  anti-thrash + measured-signal invariant; (b) reward-hacking the verifier — mitigated by F7
  immutable verifier + mutation tripwire + separation of powers; (c) unsafe autonomous merge —
  mitigated by config-driven blast-radius quarantine + auto-rollback; (d) environment coupling —
  mitigated by F8 capability detection + Phase-5.5 leak audit.
- **On merge:** regenerate the trio + goldens; update SKILL.md Phase-1 contract to the
  never-idle/never-ask semantics; `autospec validate` must pass.
