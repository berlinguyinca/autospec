# ADR 0002 — Evaluator co-evolution integration strategy

- **Date:** 2026-09-05
- **Status:** Accepted
- **Satisfies:** `docs/superpowers/specs/2026-09-05-agentic-evolution-evaluator-coevolution-handoff.md` §24 Phase 0
  ("map this spec's concepts to existing modules … write/update an ADR explaining why evolution
  semantics belong in AutoSpec control plane"), whose exit criterion is *"no coding until ownership
  and reuse plan are explicit."*
- **Consumes:** ADR 0001 D2 (14-role vocabulary), D3 (ledger is the system of record), D6 (RealWork owns benchmarks), D10 (database is global and optional).
- **Produces:** `docs/specs/2026-09-05-evaluator-coevolution-design.md` (canonical design) and
  `docs/superpowers/plans/2026-09-05-evaluator-coevolution-slice-1.md` (task plan).
- **Attaches to:** tracker #3381 (review diversity).

## Context

A Codex-generated handoff proposes adopting two papers — RQGM (arXiv:2606.26294: frozen evaluator
epochs, ε-best-belief replacement against ground-truth anchors, selective erasure) and AVO
(arXiv:2603.24517: coding agents as variation operators with a stagnation supervisor) — across every
durable `berlinguyinca/autospec*` and `InferWeave/autospec*` repository. It asks for eight subsystems:
versioned evaluators, per-slot epochs, anchor suites with a protected holdout, a challenger statistic,
selective score invalidation, candidate lineage with an agentic variation loop, a stagnation supervisor,
and an evolution policy with events.

Reconnaissance on 2026-09-05 covered `berlinguyinca/autospec` `main` at `462d3e78`,
`InferWeave/autospec-orchestrator` `main` at `4106301`, and the default branches of
`autospec-baselines`, `-constitution`, `-db`, `-gui`, `-design`, `-ui-pilot`, `-inferweave`, `-node`.
Both papers were read from the arXiv PDFs.

## Current-state findings

### Only three concepts have no owner — load-bearing

| Handoff concept | Existing owner | Gap |
|---|---|---|
| Versioned immutable evaluators | AS-AEO-001 §23.3 (results carry evaluator version; no self-grading), RealWork §52/§65, `initiative/store.rs` `is_immutable()` | no first-class evaluator object, no per-slot pinning |
| Per-slot epochs, atomic transition | none — `epoch` has zero hits in `docs/` | **entire concept** |
| Anchor suite + protected holdout | RealWork §42/§63 (buggy historical change as PR → does the reviewer catch it), `scripts/calibrate-profile.sh` | **`holdout` has zero hits repo-wide**; no known-good anchors; no access policy |
| Challenger-vs-incumbent statistic | none (only shrinkage `OBSERVATION_SHRINKAGE_K` in `aar/profile.rs`) | **entire statistic** |
| Selective score invalidation | AS-AEO-001 §25/§71 (`STALE`, `UNVERIFIED`), append-only ledgers | explicit dependency + replay metadata only |
| Candidate lineage, AVO loop | control-plane §10 tournament mode (delegated by ADR 0001 D9), `core::explore`, `schemas/autospec-explore-proposal.schema.json` | collides with `diff-guard`; see below |
| Stagnation supervisor | `2026-07-06-autospec-autonomous-platform-design.md` §F1; `feedback_capabilities_are_conductor_tiers_not_new_conductors.md` | must be a conductor tier with no separate budget |
| Evolution policy / protected surfaces | `2026-07-07-autonomy-guardrails-foundation-design.md` (`diff-guard`, `blast-radius`, `separation-of-powers`), AS-AEO-001 §12/§60/§69 | promotion thresholds and human-approval slots only |
| Events | ADR 0001 D3 "ONE ledger — extend, never fork"; `emit-event.sh` → `autospec.events.v1` | evaluator/epoch/promotion kinds |
| Hard gates outrank soft scores; producer ≠ reviewer | AS-AEO-001 §7.5/§37/§41/§46; multi-model §4; `review_policy.rs`, `review_provider.rs`; constitution doctrine 19; issue #3531 | none — consumed |

### The persistence layer is decided but unbuilt — load-bearing

ADR 0001 D5/D10 accept tokio, sqlx, and a global database. No `Cargo.toml` contains sqlx or tokio;
`crates/autospec-bench/` does not exist; `commands/benchmark.rs` is a three-line stub; #3188 is open
and unstarted. The 2026-09-01 delta spec already writes as if the database exists. The JSON-file state
layer (`state/storage.rs`, `initiative/store.rs`, `managed_project/store.rs`) is the only substrate
that exists today.

### The orchestrator cannot run a trial — load-bearing

`InferWeave/autospec-orchestrator` has two commits; issues #1–#39 are all open; `POST /executions`
is a doc comment; every harness/runtime/worktree method is `todo!()`. `ExecutionManifest` has no
free-form labels, no timeout, no required-artifacts list, a non-optional `repository`, and an
`ExecutionId` format that requires a GitHub issue. `shared-contracts.md` freezes `orchestrator-core`
except by declared additions. Issue #39 reports ~50% of Pi + Qwen runs exit 0 with no diff.

### The variation loop is what `diff-guard` exists to stop — load-bearing

`scripts/autonomous-guardrails.sh diff-guard --lane implementer` treats test files, validation scripts,
fixtures, evals, and benchmark surfaces as immutable; the verifier lane is the only bypass. Mutating a
skill prompt is a four-artifact atomic change (`SKILL.md` → `derive-trio.sh` → `gen-skill-goldens.sh`
→ three sha256 goldens) or `validate.sh` fails closed.

### `f64` is gated

`.autospec/architecture-fitness.yml` `financial_no_f64` scans `crates/**/*.rs` at threshold 0 and is
red at 81 occurrences today. No numerics crate exists (35 packages in `Cargo.lock`, no `statrs`,
`rand`, `proptest`, `uuid`, `clap`).

### Companion repositories

Constitution doctrine 19 already states "deterministic evidence outranks judgment" and "the actor that
made the work never single-handedly accepts it" (dedup commit 0.6.1 made doctrine 19 the canonical
review-law home). Baselines has rules registries under `docs/rules/*.rules.yaml` loaded by a
layout-agnostic core loader, an `evaluation-loop.md` methodology, and no fixture corpus. `autospec-db`
has one table (`autospec.events_raw`) and three views; `autospec-gui` reads schema `public` with
hardcoded table-name lists, so DB views and GUI must change together. `autospec-inferweave`
(inference plane) and `autospec-node` were not in the handoff; neither is a slice-1 dependency.

## Decisions

### D1 — Three novel things, everything else reused

Build protected-holdout anchors with an access policy, the paired challenger-versus-incumbent
statistic, and per-slot epochs with an atomic recoverable promotion. Consume — do not restate —
versioning, immutability, stale-not-delete, the append-only ledger, hard-gate precedence, and reviewer
independence. Rejected alternative: implementing the handoff's eight subsystems as proposed, which
`docs/memory/feedback_roi_check_new_components.md` would cut for lacking present-day consumers.

### D2 — Module, not crate; JSON files, not database

`crates/autospec-core/src/evaluation/` with a repo-local `.autospec/evaluation/` store. Consumes D3
(ledger is the system of record) and D10 (database is global and optional). When #3188 lands sqlx,
evaluation projections are additive views, never authority. Rejected alternatives: a new
`autospec-evolution` repository or crate (a second consistency boundary with no service to justify
it); building on the unbuilt database.

### D3 — Integer-only statistic

`BB_ε` is the smallest `p` in parts per million with `P[Bin(S+F+1, p) ≥ S+1] ≥ ε`, computed with a
`u128` fixed-point term recurrence from the binomial mode and bisection. Keeps `financial_no_f64` at
zero new occurrences, adds no dependency, and is bit-identical across Linux, macOS, Windows, and
FreeBSD CI. Verified against two independent reference computations to 1 ppm. Rejected alternatives:
`f64` with a gate exclusion (the gate's intent is money paths, but the gate is already red and an
exclusion hides drift); a numerics crate (an ADR-level dependency decision for one function).

### D4 — Fail closed; ties never promote

Below the incumbent's lower bound → `Rejected`; at or above but under `minimum_margin`, or any
`Unavailable` verdict, or fewer than `minimum_cases` → `Inconclusive`; any protected-subset ceiling
breach or regression beyond tolerance → `Rejected`. Only `Qualified` can promote, and slots listed in
policy also need a human approval. This is RQGM's "ties favour the incumbent" plus AutoSpec's
"gather more evidence" (handoff §9.4).

### D5 — Learned-judge execution is a seam, not a slice-1 deliverable

`VerdictSource` has one implementation, `RecordedVerdicts`. Live judging through the executor
abstraction (PR #3196) is slice 2; through the orchestrator is slice 2b, gated on orchestrator issues
#2, #4, #6–#8, #11–#13, #15, #20, #23, #25 and a `shared-contracts.md` amendment.

### D6 — Constitution is amended, not restated

Add only three clause families to doctrine 19: epoch-immutable evaluator versions, evidence-gated
replacement against protected anchors, append-only evaluator history with spend ceilings. Minor bump
0.6.1 → 0.7.0. Baselines gains `docs/rules/evaluator-qualification.rules.yaml` and one synthetic
fixture pack. Both are manual PRs filed after this spec merges.

### D7 — Attach to tracker #3381, not a parallel track

Evaluator issues are children of "review diversity — a reviewer that does not share the implementer's
priors". `RuntimeProvenance` reuses the attestation fields of #3531 (which model, what effort,
independent). Rejected alternative: a new epic, which would repeat the #3162 family's seven
`parked-superseded` closures.

## Consequences

- Slice 1 (the plan) is decomposable now; it has no unbuilt dependency.
- Slice 2 waits on a callable review-only dispatch; slice 2b on the orchestrator; slice 3 on a
  `diff-guard` lane for mutable surfaces; slice 4 on stable journal kinds; slice 5 on slice 2.
- No new crate, dependency, database, bats suite, or JSON schema file is introduced by slice 1.

## Open items

1. Whether `routing_policy_digest` is derived from `aar::profile::ModelProfileRegistry` (Rust) or
   `~/.autospec/model-profiles.yml` (shell). Slice 1 accepts a caller-supplied digest; slice 2 decides.
2. Whether the orchestrator's `shared-contracts.md` amendment is filed from this repository or from
   the orchestrator's own issue flow.
