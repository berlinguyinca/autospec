# Runbook: discovery sweep (features, problems & self-leverage)

The operator-runnable, one-shot form of the methodology that
`/autospec-explore` runs perpetually. Use this when you want a single audit
pass without arming the loop. It is kept in **lockstep** with
`docs/specs/2026-06-15-autospec-explore-discovery-enhance.md` — both describe
the same five discovery tracks, the adversarial verify stage, and pattern
synthesis. When you change one, change the other (enforced by
`check_autospec_explore_discovery_contract` in `autospec validate`).

## How to run

Paste the prompt below into a fresh Claude session at the repo root, or invoke
`/autospec-explore` for the autonomous, continuous version. The five tracks map
1:1 onto explore's researchers; the verify/severity/ROI/synthesis steps map
onto its aggregator.

---

## Prompt

```
You are auditing the autospec codebase to find (a) MISSING features, (b) QUALITY
and STABILITY problems, and (c) opportunities to AUTOMATE work I currently do by
hand. autospec runs autonomously with auto-merge, so a bug that ships green is
worse than a missing feature — weight accordingly. Your job is not just a report:
it's to produce verified, ready-to-act findings that make running this project
easier for me over time.

## Phase 0 — Calibrate
Confirm scope and depth with me, then set:
  - a CONFIDENCE BAR: every finding needs grounding evidence; speculation is
    rejected. False positives are the known failure mode of this repo — be strict.
  - SEVERITY model: silent-wrong > correctness > stability > operability >
    feature > nicety. Rank by blast radius through auto-merge + lock-step first,
    then user/workflow impact, then effort.

## Phase 1 — Ground truth (read-only, parallelize)
Map, via parallel Explore agents:
  1. INTENDED surface — capabilities promised in AGENTS.md, CLAUDE.md,
     docs/specs/*.md, docs/superpowers/specs/, each skills/*/SKILL.md description.
  2. ACTUAL surface — what code/scripts/skills/validate.sh actually implement.
  3. The delta. Read docs/memory/MEMORY.md + relevant docs/memory/*.md FIRST;
     do not re-report tracked items (e.g. #420, #421) — but DO mine them for
     recurring themes (see Phase 2.5).

## Phase 2 — Multi-track discovery (run all five tracks in parallel)

TRACK A — Feature delta (internal): things promised/implied/required by a
  documented workflow that have no working implementation.

TRACK B — External/ecosystem research: use web search/fetch to scan comparable
  agentic/codegen/spec-driven pipelines, orchestration frameworks, relevant
  papers/standards (LLM eval, agent guardrails, autonomous-merge policy), and
  public changelogs/issue trackers of similar tools. Build a capability matrix
  (tools × capabilities) so gaps are visible. Capture SOURCE URLs. Treat ideas
  as inspiration, not mandates — adapt to this repo's philosophy.

TRACK C — Quality & resilience (the four lenses):
  - Test-of-tests: tests that can't fail — self-consistent fixtures built with
    the SUT's own derivation expr, assertion-free "it ran" tests, missing
    negative-path pairs. (The transcript-slug bug shipped green this way.)
  - Invariant↔guard coverage: every claimed invariant (lock-step, single-source
    palette, auto-merge gate, closeout contract) must have BOTH a validate.sh
    check AND a test. Flag any with neither.
  - Failure-injection: what corrupts/loses work when a run is killed mid-step?
    Non-idempotent steps, partial-state, un-pushed-work loss, monitor
    silent-exit, heartbeat cross-repo collision, subagent-cwd contamination,
    shared-lock races, jq regex-injection on host-derived values.
  - Determinism & cost: LLM-driven steps that should be deterministic tools
    (#421); phases that burn disproportionate tokens/wall-clock.

TRACK D — Dogfooding (mine actual behavior, not just code):
  - Read live state under ${AUTOSPEC_STATE_DIR:-$HOME/.autospec}: run-state,
    failure ledgers, heartbeats, explore-loop.json, run-summary.md,
    explore-summary.md, and the outcome ledger. Where do runs retry, stall,
    silent-exit, or fail? What ships then gets reverted? (If the dir is absent,
    say so and skip — don't fabricate.)
  - Dead surface: skills/flags/branches never invoked.
  - Git churn & revert archaeology: high-churn files and bug-fix commit clusters
    = fragile hotspots; recently-reverted areas = instability.
  - Issue/PR archaeology: recurring labels, reopened issues, closed-without-merge,
    promised-in-issue-but-never-done.
  Redact host-specific absolute paths to ~/- or repo-relative form in findings.

TRACK E — Self-leverage ("help me help myself"):
  - Find every point where a HUMAN decision/intervention is still required
    (prompts, manual recovery, hand-curation, relaunch-after-silent-exit).
  - For each: could it be auto-resolved, defaulted, or surfaced better? Per the
    autonomy-scope rule, low-stakes decisions should auto-resolve; only
    run/defer/refine + destructive-remote actions should reach me.
  - Adversarial onboarding: walk the clean-install + first-command path as a
    hostile naive user (install.sh has shipped runtime-lib-drop crashes that
    ship-completeness missed). Note every trip hazard.

DOMAIN SPECIALISTS (layer over the tracks): first detect this repo's domain
from dependency manifests + README/AGENTS keywords (cite the evidence). If a
clear domain exists, ALSO run 1–3 specialist lenses appropriate to it (e.g. a
trading repo → quant-strategy + market-risk + exchange-integration; a
healthcare app → hipaa-compliance + clinical-safety) and let each propose
domain-specific gaps. If you find no clear domain, skip specialists. Ask me to
confirm or adjust the specialist roster and count before you rely on it.

GROUNDING (all tracks): internal findings cite file:line for BOTH "promised/
expected here" and "missing/broken here". External findings cite source URL AND
a file:line proving the repo doesn't already do it. Lone TODO/FIXME greps and
unfit external ideas are noise — reject.

## Phase 2.5 — Pattern synthesis (root-cause, not whack-a-mole)
Cluster findings + recurring memory themes. Anything that has bitten ≥2 times
(e.g. the family of bash gotchas, the "shipped green for months" class) gets a
STRUCTURAL fix proposal — a lint/shellcheck gate, a fixture-derivation rule, a
new validate.sh family — instead of N point patches. Name the class and the
single guard that would have caught all instances.

## Phase 3 — Adversarially verify, then rank
For each candidate, spawn an independent skeptic prompted to REFUTE it
(default to "not a real gap" under uncertainty). Drop anything refuted. Then
apply the ROI gate: every survivor needs a NAMED consumer who benefits today —
reject speculative features. Rank survivors by the Phase 0 severity model.
Present top 8–12 as a table: title, track/source(+URL), severity, blast radius,
effort, named consumer, evidence (file:line). STOP for my picks.

## Phase 4 — Close the loop (after I pick which to file)
For each approved finding, draft a GitHub issue body in this repo's normal
decompose→run shape (intent, surface, acceptance checks, external inspiration
cited) AND file it with `gh issue create` once I confirm the batch. Goal: I can
run them via /autospec-run, not re-process them.

## Phase 5 — Integrate (only items I explicitly approve to build)
  - Implement against a short design spec (docs/specs/*.md). Respect trio-skill
    derivation: editing any SKILL.md means re-deriving codex/opencode prompts
    AND regenerating goldens, or validate.sh fails closed.
  - Add/extend tests + validate.sh so the new capability/guard is enforced.
  - Authoring and review stay separate passes; never self-approve — run a
    code-reviewer/verifier pass. Regression-risk to lock-step/auto-merge is a
    BLOCKING gate, not a score column.

## Phase 6 — Make it repeatable & compounding
Capture this sweep (all five tracks + verify) as a re-runnable checklist/script
so it reruns after future changes. Write new gaps, dead surface, useful external
sources, and extracted root-cause patterns into docs/memory/ so each run starts
smarter than the last.

## Constraints
- Read-only until Phase 5 approval (Phase 4 files issues only after I confirm).
- Small-LLM operability (60–120k ctx), correctness >> speed, conservative
  guardrails, lock-step + auto-merge guarantees preserved.
- External ideas are inspiration, not mandates.

Deliver Phases 1–3 as a report. Wait for my selection before Phase 4+.
```

---

## Track ↔ explore researcher map

| Runbook track | explore researcher(s) |
|---|---|
| A — Feature delta | `spec-vs-code`, `source-analysis` |
| B — External/ecosystem | `internet` |
| C — Quality & resilience | `quality-resilience` (new), `dependency-health` |
| D — Dogfooding | `dogfooding` (new), `prior-reports`, `open-issues` |
| E — Self-leverage | `self-leverage` (new) |
| 2.5 — Pattern synthesis | aggregator pattern-synthesis stage |
| 3 — Verify + severity + ROI | aggregator verify stage + severity-first rank + ROI gate |
