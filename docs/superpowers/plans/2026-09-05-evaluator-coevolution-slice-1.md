# Evaluator Co-Evolution, Slice 1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give AutoSpec versioned, immutable learned evaluators pinned into per-slot epochs, a protected labeled anchor suite, a deterministic challenger-versus-incumbent qualification statistic, and a crash-safe promotion transaction that marks displaced scores stale without deleting history.

**Architecture:** One new cohesive module `crates/autospec-core/src/evaluation/` (pure domain types, integer-only statistics, and a repo-local file store under `.autospec/evaluation/` modelled on the existing `InitiativeStore` immutable-artifact layout and the `managed_project` hash-chained journal), exposed through two new hand-rolled CLI groups `autospec evaluator` and `autospec anchor`. Learned-judge execution is behind a `VerdictSource` trait whose only slice-1 implementation replays recorded verdict files, so every protected state transition is deterministic and testable without a model. No orchestrator, database, async runtime, or new crate dependency is introduced.

**Tech Stack:** Existing Rust workspace (`serde`, `serde_json`, `sha2` via `autospec_core::autonomous::waterfall::sha256_hex`), std filesystem with tmp+rename+fsync, inline `#[cfg(test)]` unit tests plus `crates/*/tests/*.rs` integration tests. No `f64` (the `financial_no_f64` architecture gate scans every `crates/**/*.rs`). No `tokio`, `sqlx`, `clap`, `uuid`, `proptest`.

**Spec:** `docs/superpowers/specs/2026-09-05-agentic-evolution-evaluator-coevolution-handoff.md` (the Codex handoff, verbatim), reconciled in Part A below. The reconciliation wins where they disagree. Canonical in-repo design and ADR are produced by Task 1.

## Global Constraints

Copied from the handoff spec and from the repository's own gates. Every task's requirements implicitly include this section.

- "Do not create a new microservice/repository." (handoff §31) — everything lands in `berlinguyinca/autospec`, plus doctrine text in `autospec-constitution` and a seed pack in `autospec-baselines`.
- "Do not move promotion policy into autospec-orchestrator." (§31) — the orchestrator is a two-commit scaffold today (all 39 implementation issues open, `POST /executions` is a doc comment); slice 1 does not touch it.
- "Do not make PostgreSQL required for core correctness." (§31) — ADR 0001 D3: "The JSONL ledger is the system of record; any database is a projection." `sqlx`/`tokio` are accepted by D5/D10 but **not present in any `Cargo.toml`**; slice 1 uses the existing JSON-file state layer only.
- "Do not allow evaluator prompt updates in place." (§31) — evaluator versions are `create_new` files; a second write to the same version is an `Immutable` error.
- "Do not delete historical evaluations on epoch replacement." (§31) — records gain `active_ranking_status` and a `ranking_history`; original judgment fields never change.
- "Do not let candidate agents inspect holdout labels." (§31) — `AnchorSuite::view(AccessRole::Mutation)` strips `expected_label` from `protected_holdout` cases and drops `quarantine` cases.
- "Do not claim statistical significance without implementing and testing the statistic used." (§31) — `statistics.rs` implements `BB_ε = ε-quantile of Beta(1+S, 1+F)` in integer arithmetic and is tested against reference values computed by two independent methods (binomial tail in log space, and Simpson integration of the Beta density).
- "Promotion commands MUST fail closed if evidence is incomplete." (§16.1) — any `Unavailable` verdict, missing incumbent, policy-digest mismatch, or missing human approval returns `EvaluationErrorKind::FailClosed`.
- Paper facts the implementation must match (arXiv:2606.26294 §3.1, §3.5, verified from the PDF): `BB_ε(a) = I⁻¹_ε(1+S_a, 1+F_a)`, default `ε = 0.05`; at an epoch boundary "the largest anchor BB_ε is frozen as the next-epoch evaluator, with ties favoring the incumbent"; selective erasure removes "only the utility history attached to the displaced slot while preserving all unrelated information".
- Repository gates: `financial_no_f64` (`.autospec/architecture-fitness.yml`, pattern `\bf64\b`, threshold 0, currently red at 81 — **add zero new occurrences**); file-size ratchet (`.github/workflows/file-size-ratchet.yml`, 600 LOC soft / 2000 hard per `.rs` file — every new file stays under 600); `rust_core_cli_direction` (core never depends on CLI).
- Dependency policy: every dependency in `crates/*/Cargo.toml` is `=`-pinned with a `# why:` comment; this plan adds **none**.
- Tests: `cargo test --workspace --no-fail-fast` (AGENTS.md: without `--no-fail-fast` one failing binary hides the rest). No new `.bats` suites (a new suite outside `bats_registration_baseline.rs` fails validation).
- Git hygiene (AGENTS.md): branch `feat/<slug>`, conventional commits, never push to `main`, all git-mutating steps in a linked worktree created by `scripts/worktree-guard.sh`, never bypass git hooks, never amend.
- Timestamps are `u64` unix seconds supplied by the caller; core never reads a clock (existing convention in `rag/budget.rs`, `evidence/mod.rs`).
- Digests are lowercase SHA-256 hex, 64 chars, computed over NUL-separated fields via `sha256_hex` (existing convention in `review_evidence.rs`, `managed_project.rs`).

---

## Part A — Phase 0 reconciliation (handoff §24 Phase 0, §32 Steps A–C)

Reconnaissance was performed on 2026-09-05 against `berlinguyinca/autospec` `main` at `462d3e78`, `InferWeave/autospec-orchestrator` `main` at `4106301`, and the current default branches of every `berlinguyinca/autospec-*` repository via `gh`. Findings that changed the plan are marked **load-bearing**.

### A.1 Repository enumeration

| Repository | Durable? | Role | Activity |
|---|---|---|---|
| `berlinguyinca/autospec` | yes | control plane, source of truth | daily (PR #3506 on 2026-09-04) |
| `InferWeave/autospec-orchestrator` | yes | execution plane | **two commits total**; issues #1–#39 all open |
| `berlinguyinca/autospec-baselines` | yes | reusable packs + gate registries | dormant since 2026-07-10 |
| `berlinguyinca/autospec-constitution` | yes | doctrine, v0.6.1 | dormant since 2026-07-10 |
| `berlinguyinca/autospec-db` | yes | optional Postgres telemetry (Go) | dormant since 2026-07-10 |
| `berlinguyinca/autospec-gui` | yes | read-only Next.js dashboard | dormant since 2026-08-11 |
| `berlinguyinca/autospec-design` | yes | `DESIGN.md` contract + rules pack | single commit 2026-08-05 |
| `berlinguyinca/autospec-ui-pilot` | yes | UI-gate calibration corpus | dormant since 2026-08-06 |
| `berlinguyinca/autospec-inferweave` | yes, **not in handoff** | inference plane (private) | most active companion |
| `berlinguyinca/autospec-node` | yes, **not in handoff** | node hardware extraction (private) | irrelevant to this work |
| `autospec-e2e-listener-*`, `autospec-e2e-handoff-*` | no | generated E2E fixtures | excluded, as the handoff says |

### A.2 Concept-to-existing-code map (handoff §32 Step A)

| Handoff concept | Existing owner | Decided already | Gap | Slice-1 owner |
|---|---|---|---|---|
| Versioned immutable evaluators (§7.1, §5.4) | AS-AEO-001 §23.3 (results carry evaluator version; "benchmarks shall not rely only on model self-grading"); RealWork §52/§65 (content-addressed, immutable corpus); `initiative/store.rs` `is_immutable()` pattern | versioning + immutability | **no first-class evaluator object, no per-slot pinning** | `evaluation/evaluator.rs` |
| Per-slot epochs, atomic transition (§7.2, §9.5) | none (`epoch` has zero hits in `docs/`) | — | **entire concept** | `evaluation/epoch.rs`, `store/transition.rs` |
| Anchor suite + protected holdout (§7.3, §11.3) | RealWork §42 review benchmarks (buggy historical change presented as PR → does the reviewer catch it), §63 validity, §24 qualification; `scripts/calibrate-profile.sh` replays merged issues per role | known-bad anchors, "defect recall with low false-positive noise" | **`holdout` has zero hits repo-wide; no known-good anchors; no access policy** | `evaluation/anchor.rs` |
| Challenger vs incumbent statistic (§9.3) | none (only Bayesian shrinkage `OBSERVATION_SHRINKAGE_K` in `aar/profile.rs` and smoothed ledgers in bash) | conservatism principle ("adjusted — never replaced") | **entire statistic** | `evaluation/statistics.rs`, `qualification.rs` |
| Selective active-score invalidation (§10) | AS-AEO-001 §25/§71 (`STALE`, `UNVERIFIED`), append-only ledgers (`explore-ledger.sh --update-outcome` appends a new copy) | stale-not-delete is doctrine | **selective by explicit dependency; replay metadata** | `evaluation/record.rs` |
| Candidate manifests + lineage, AVO loop (§7.5, §12) | control-plane spec §10 tournament mode (delegated by ADR D9 to multi-model §4 + D2), `schemas/autospec-explore-proposal.schema.json`, `core::explore` | candidate generation, fairness, losing candidates preserved | collides with `diff-guard` protected kernel — see A.4 | **deferred to slice 3** |
| Stagnation supervisor (§14) | `2026-07-06-autospec-autonomous-platform-design.md` §F1 ("convergence-stop is impossible"), `docs/memory/feedback_capabilities_are_conductor_tiers_not_new_conductors.md` | must be a conductor tier, no separate budget | quality-plateau signal | **deferred to slice 3** |
| Evolution policy, protected surfaces (§7.9, §5.1) | `2026-07-07-autonomy-guardrails-foundation-design.md` (`diff-guard`, `blast-radius`, `mutation-guard`, `separation-of-powers`), AS-AEO-001 §12/§60/§69 | protected kernel is live in `autonomous-premerge-gate.sh` | promotion thresholds + human-approval slots only | `evaluation/policy.rs` (promotion policy only) |
| Events (§18) | ADR D3 "ONE ledger — extend, never fork"; `emit-event.sh` → external `autospec-db` (`autospec.events.v1`, `event_uuid` idempotency, additive-only) | envelope + idempotency | evaluator/epoch/promotion kinds | `store/journal.rs` (local), DB kinds in slice 4 |
| Hard gates outrank soft scores (§5.2) | AS-AEO-001 §7.5, §37, §41, §46; multi-model §29 Phase A/B; constitution doctrine 19 ("the gate wins") | decided, verbatim | none | reused, not re-implemented |
| Producer ≠ reviewer (§5.3) | multi-model §4.1–§4.6; `autonomous/review_policy.rs` + `executor_bridge/review_provider.rs` (`providers_are_diverse`, unknown ⇒ not diverse); issue #3531 attestation shape; tracker #3381 | decided and partly built | evaluator record must carry provenance | `RuntimeProvenance` in `record.rs` mirrors #3531 fields |
| Model routing digest (§5.4) | `aar/profile.rs` `ModelProfileRegistry::new(version, ..)`; shell `model-profiles.yml` unwired to Rust | two disconnected surfaces | choose which to digest | evaluator pins a caller-supplied `routing_policy_digest`; slice 2 derives it from `ModelProfileRegistry` |
| Orchestrator trial labels (§16.2) | orchestrator `ExecutionManifest` has no free-form labels, no timeout, no required-artifacts, `repository` is not optional, `ExecutionId` format requires an issue | nothing runs end-to-end | slice 2 | `VerdictSource` trait seam only |
| Benchmark subsystem (`autospec bench`) | RealWork spec authoritative (ADR D6); `commands/benchmark.rs` is a 3-line stub; `crates/autospec-bench/` absent; never decomposed | authority decided, nothing built | do not build a second benchmark | anchor suites are **evaluator** ground truth, not task benchmarks; documented as consumers of RealWork §42 when it exists |

### A.3 Decisions (to be recorded in `docs/decisions/0002-…` by Task 1)

- **D1 — Three novel things, everything else reused.** The genuine contribution is (i) protected holdout anchors with an access policy, (ii) the paired challenger-versus-incumbent qualification statistic, and (iii) per-slot epochs with an atomic, recoverable promotion. Versioning, immutability, stale-not-delete, append-only ledgers, hard-gate precedence, and reviewer independence already have owners and are consumed, not restated. (Maintainer rule: `docs/memory/feedback_roi_check_new_components.md` — name the consumer that benefits today.)
- **D2 — Module, not crate; JSON files, not database.** `crates/autospec-core/src/evaluation/` with a repo-local `.autospec/evaluation/` store. Consumes ADR 0001 D3 (ledger is the system of record) and D10 (database is global and optional). When #3188 lands sqlx, `evaluation` projections are additive views, never authority.
- **D3 — Integer-only statistic.** `BB_ε` is computed as the smallest `p` in parts-per-million such that `P[Bin(S+F+1, p) ≥ S+1] ≥ ε`, using a `u128` fixed-point term recurrence from the binomial mode. This keeps the `financial_no_f64` gate at zero new occurrences and makes results bit-identical across platforms (Linux, macOS, Windows, FreeBSD are all in CI).
- **D4 — Fail closed everywhere; ties do not promote.** A challenger whose `BB_ε` is below the incumbent's is `Rejected`; at or above but under `minimum_margin` (or with any `Unavailable` verdict, or fewer than `minimum_cases`) is `Inconclusive` and needs more evidence; any protected-subset ceiling breach or regression beyond tolerance is `Rejected`. Only `Qualified` can be promoted, and only with a human approval for slots listed in policy.
- **D5 — Learned judge execution is a seam, not a slice-1 deliverable.** `VerdictSource` has one implementation, `RecordedVerdicts` (JSON). Live judging through `executor_bridge` (issue #3172's executor abstraction) is slice 2; through the orchestrator is slice 2b, gated on orchestrator issues #2–#25 landing.
- **D6 — Constitution is amended, not restated.** Doctrine 19 already says the gate wins and the producer is never the sole approver. Only three new clause families are added: epoch-immutable evaluator versions, evidence-gated replacement against protected anchors, and append-only evaluator history with spend ceilings. Minor bump 0.6.1 → 0.7.0.
- **D7 — Attach to tracker #3381, not a parallel track.** The evaluator work is filed as children of "review diversity — a reviewer that does not share the implementer's priors". `RuntimeProvenance` reuses the attestation fields defined in #3531 (`which model, at what effort, independent`).

### A.4 Findings that gate later slices (load-bearing)

1. **The variation loop is what `diff-guard` exists to stop.** `scripts/autonomous-guardrails.sh diff-guard --lane implementer` treats "test files, validation scripts, fixtures, evals, and benchmark surfaces" as immutable; the verifier lane is the only bypass. Slice 3 must declare its mutable-surface set as an explicit lane with its own provenance, or every candidate PR is blocked at `autonomous-premerge-gate.sh`.
2. **Mutating a skill prompt is a four-artifact atomic change** (`SKILL.md` → `derive-trio.sh --in-place` → `gen-skill-goldens.sh` → three sha256 goldens). A prose-only mutation is unmergeable by construction.
3. **The orchestrator cannot run a trial today** and its `shared-contracts.md` freezes `orchestrator-core` except by declared additions. Slice 2b needs: optional `repository` or an `inputs` mount list, free-form `labels: BTreeMap<String,String>` kept out of Docker labels, `timeout_seconds`, `required_artifacts`, `exit_code`, and an `ExecutionId` form without an issue segment.
4. **Issue #39 in the orchestrator** reports ~50% of Pi + Qwen runs exit 0 with no diff. A trial outcome must distinguish `NO-OUTPUT`/`STALLED` from success; `Verdict::Unavailable` in this plan is the placeholder that fails closed.
5. **Nearest open issues:** #3531 (reviewer attestation: `ReviewerAuthority`, `GateKind`), #3536 (differential gates + baseline debt), #3532 §2 ("unmeasured must render as `unknown`, never `0`"), #3381 (tracker). None are labelled `auto-implement` yet.

### A.5 What this plan deliberately does not do

- No `evolve` CLI, candidate manifests, lineage, Thompson sampling, clade metaproductivity, stagnation supervisor (slice 3+).
- No orchestrator, DB migration, or GUI change (slices 2b, 4).
- No live LLM judge invocation (slice 2).
- No new bats suites, schemas under `schemas/` (validation is structural presence only; JSON contracts are asserted by Rust tests), or new dependencies.
- No GitHub issues are created by this plan; Task 17 describes the decomposition for `/autospec-split` once the canonical spec is on `origin/main` (`docs/memory/feedback_autospec_split_origin_main_gate.md`).

---

## Part B — File structure

All new Rust files stay under 600 lines. Responsibilities are one per file.

```text
crates/autospec-core/src/lib.rs                       modify: add `pub mod evaluation;`
crates/autospec-core/src/evaluation/
  mod.rs            module list, EVALUATION_SCHEMA_VERSION, re-exports
  error.rs          EvaluationError { kind, message }, From<io::Error>, From<serde_json::Error>
  ids.rs            EvaluatorSlot, EvaluatorVersionRef ("architecture@12"), EpochId ("epoch-000001"), string ids
  digest.rs         Digest (64-hex), of_bytes / of_parts (NUL-joined)
  statistics.rs     Ppm, binomial_upper_tail, best_belief_ppm (integer-only)
  evaluator.rs      EvaluatorKind, Provenance, EvaluatorDefinition + definition_digest + validate
  anchor.rs         ProtectedLabel, Severity, AnchorVisibility, AccessRole, ProtectedSubset, AnchorCase, AnchorSuite (view / verify_artifacts / suite_digest)
  policy.rs         PromotionPolicy (+ Default, validate, policy_digest, JSON)
  qualification.rs  Verdict, PairedCaseResult, pair_results, EvaluatorTally, SubsetOutcome, ChallengerVerdict, QualificationReport, qualify, ChallengerTrial, VerdictSource, RecordedVerdicts
  epoch.rs          EvaluatorEpoch (genesis / version_of / successor)
  record.rs         Independence, RuntimeProvenance, ActiveRankingStatus, RankingTransition, EvaluationRecord, stale_candidates
  promotion.rs      ApprovalKind, Approval, PromotionState, PromotionEvent, plan_promotion, plan_pin
  store/mod.rs      EvaluationStore struct + open/init + read accessors
  store/layout.rs   EvaluationLayout path helpers (.autospec/evaluation/…)
  store/io.rs       atomic_write, append_synced_line(fail_after), write_immutable_json, read_json
  store/journal.rs  JournalEvent, Journal (hash chain, checkpoint, dedup keys)
  store/transition.rs  TransitionStep, Faults, promote/pin/run_transition/recover_pending_promotions
crates/autospec-core/tests/
  evaluation_statistics.rs      reference-value tests for best_belief_ppm
  evaluation_registry.rs        immutability, digests, anchor access, policy digest
  evaluation_qualification.rs   qualified / rejected / inconclusive / protected-regression fixtures
  evaluation_transition.rs      crash matrix: inject after every step, reopen, converge to one active epoch
  fixtures/evaluation/          anchor suite v1 (40 cases), recorded verdicts (v1, v2, v2-regress, v3-tie), evaluator definitions, records
crates/autospec-cli/src/commands/mod.rs               modify: register `evaluator` and `anchor`
crates/autospec-cli/src/commands/evaluator.rs         dispatch + help
crates/autospec-cli/src/commands/evaluator/args.rs    option parsing, --root, --json, now()
crates/autospec-cli/src/commands/evaluator/registry_cmds.rs   init / register / list / show / pin
crates/autospec-cli/src/commands/evaluator/epoch_cmds.rs      epoch current / history
crates/autospec-cli/src/commands/evaluator/challenger_cmds.rs challenger run / inspect
crates/autospec-cli/src/commands/evaluator/promote_cmds.rs    promote, record add / list
crates/autospec-cli/src/commands/anchor.rs            register / list / show --role / verify
crates/autospec-cli/tests/evaluator_commands.rs       end-to-end demo from handoff §32 Step F
docs/decisions/0002-evaluator-coevolution-integration-strategy.md   ADR (Task 1)
docs/specs/2026-09-05-evaluator-coevolution-design.md              canonical spec (Task 1)
docs/cli-reference.md, docs/CONFIG_REFERENCE.md, docs/architecture.md  docs (Task 14)
autospec-constitution: docs/19-review-and-critique-doctrine.md, CONSTITUTION.md, CHANGELOG.md, schemas/constitution.schema.json (Task 15)
autospec-baselines: docs/rules/evaluator-qualification.rules.yaml, docs/rules/anchor-fixtures/architecture-v1/ (Task 16)
```

Store layout on disk (repo-local, created by `autospec evaluator init`):

```text
.autospec/evaluation/
  policy.json                          immutable once written (re-init fails)
  evaluators/<slot>/v<N>.json          immutable
  anchors/<suite-id>/v<N>.json         immutable; artifact_ref paths are repo-relative
  epochs/epoch-<NNNNNN>.json           immutable
  current.json                         { "schema": 1, "epoch_id": "epoch-000001" }   (the single atomic pointer)
  trials/<trial-id>.json               immutable
  promotions/<promotion-id>.json       state pending → committed (atomic rewrite)
  records/<evaluation-id>.json         judgment fields immutable; ranking status + history appended
  events.jsonl                         hash-chained, dedup keys
  events.checkpoint.json               { "schema": 1, "high_watermark": N, "digest": "<64 hex>" }
```

---
## Part C — Tasks (berlinguyinca/autospec)

Work in a linked worktree on branch `feat/evaluator-coevolution-slice-1` (`bash scripts/worktree-guard.sh create --branch feat/evaluator-coevolution-slice-1`). Commit after every task. Run `cargo test -p autospec-core --lib evaluation` for fast feedback and `cargo test --workspace --no-fail-fast` before each commit.

### Task 1: Phase 0 ADR and canonical spec

**Files:**
- Create: `docs/decisions/0002-evaluator-coevolution-integration-strategy.md`
- Create: `docs/specs/2026-09-05-evaluator-coevolution-design.md`
- Modify: `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md` (append one line under `## Open items`)

**Interfaces:**
- Produces: the two documents every later task cites as `Source spec`. Nothing in code depends on this task, but `/autospec-split` (Task 17) requires the spec on `origin/main`.

- [ ] **Step 1: Write the ADR** with exactly this header and the decisions from Part A.3, findings from Part A.4, and the concept map from Part A.2 (copy the tables verbatim):

```markdown
# ADR 0002 — Evaluator co-evolution integration strategy

- **Date:** 2026-09-05
- **Status:** Accepted
- **Satisfies:** `docs/superpowers/specs/2026-09-05-agentic-evolution-evaluator-coevolution-handoff.md` §24 Phase 0
  ("map this spec's concepts to existing modules … write/update an ADR explaining why evolution
  semantics belong in AutoSpec control plane"), whose exit criterion is *"no coding until ownership
  and reuse plan are explicit."*
- **Consumes:** ADR 0001 D2 (14-role vocabulary), D3 (ledger is the system of record), D6 (RealWork owns benchmarks), D10 (database is global and optional).
- **Attaches to:** tracker #3381 (review diversity).

## Context
<Part A.1 table>

## Current-state findings
<Part A.2 table; A.4 items 1–5, each marked **load-bearing**>

## Decisions
### D1 — Three novel things, everything else reused
...
### D7 — Attach to tracker #3381, not a parallel track

## Consequences
- Slice 1 (this ADR's plan) is decomposable now: it has no unbuilt dependency.
- Slice 2 (live judge execution) waits on the executor abstraction path from #3172 being callable for a review-only role.
- Slice 2b (orchestrator trials) waits on orchestrator issues #2, #4, #6–#8, #11–#13, #15, #20, #23, #25 and a `shared-contracts.md` amendment adding `labels`, `timeout_seconds`, `required_artifacts`, `exit_code`.
- Slice 3 (AVO variation) waits on a `diff-guard` lane definition for mutable surfaces and on trio-golden regeneration being callable from the loop.

## Open items
1. Whether `routing_policy_digest` is derived from `aar::profile::ModelProfileRegistry` (Rust) or `~/.autospec/model-profiles.yml` (shell). Slice 1 accepts a caller-supplied digest.
```

- [ ] **Step 2: Write the canonical spec** at `docs/specs/2026-09-05-evaluator-coevolution-design.md` using the repo's section convention (Goal · Team personality · Review counter-team · Architecture · Data model · Error handling · Testing · Acceptance criteria · Critical risk check). Content: Part A.3 decisions as the Architecture, Part B as the Data model and store layout, the `qualify()` rules from Task 7 as the promotion semantics, the transition steps from Task 11, and this acceptance list (checkbox form):

```markdown
## Acceptance criteria
- [ ] `autospec evaluator register` refuses to overwrite an existing `<slot>/v<N>.json` (exit 2, message names the file).
- [ ] `autospec evaluator epoch current` prints one version per slot and the epoch's policy digest.
- [ ] `autospec evaluator challenger run` on the fixture suite yields `qualified` for v1→v2, `rejected` for v1→v2-regress, `inconclusive` for v1→v3-tie, and refuses a verdict file with an `unavailable` entry.
- [ ] `autospec evaluator promote` on a `qualified` trial for slot `architecture` fails closed without `--approve --actor`, and with it creates `epoch-000002`, marks every active `architecture@1` record `stale_for_active_ranking`, and leaves the record's original outcome untouched.
- [ ] Killing the process after any of the six transition steps and re-running `autospec evaluator epoch current` shows exactly one active epoch and a committed promotion.
- [ ] `autospec anchor show <suite> --role mutation` prints no `expected_label` for `protected_holdout` cases and omits `quarantine` cases.
- [ ] `autospec anchor verify <suite>` exits non-zero and names the case when a fixture file's digest differs from the suite.
- [ ] `cargo test --workspace --no-fail-fast` passes; `bash scripts/architecture-fitness.sh run --registry .autospec/architecture-fitness.yml` reports `forbidden_f64_occurrences` unchanged (81).
```

Add a `**Related:**` list linking `docs/specs/2026-08-16-autonomous-engineering-organization-design.md` §23.3/§24/§25, `docs/specs/2026-08-16-repository-derived-real-work-benchmark-design.md` §42/§63, `docs/specs/2026-08-16-multi-model-engineering-team-design.md` §4, `docs/specs/2026-07-07-autonomy-guardrails-foundation-design.md`, and the handoff file.

- [ ] **Step 3: Append to ADR 0001 Open items:** `4. Evaluator co-evolution (handoff 2026-09-05) is reconciled in ADR 0002; its Epic 3/4/6 touch points consume D3, D6, D10.`

- [ ] **Step 4: Commit**

```bash
git add docs/decisions/0002-evaluator-coevolution-integration-strategy.md docs/specs/2026-09-05-evaluator-coevolution-design.md docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md docs/superpowers/specs/2026-09-05-agentic-evolution-evaluator-coevolution-handoff.md docs/superpowers/plans/2026-09-05-evaluator-coevolution-slice-1.md
git commit -m "docs: evaluator co-evolution ADR 0002, canonical spec, and slice-1 plan"
```

---

### Task 2: Module skeleton, errors, ids, digests

**Files:**
- Modify: `crates/autospec-core/src/lib.rs` (add `pub mod evaluation;` in alphabetical position after `pub mod evidence;` — check the existing list; `error.rs` is a sibling, so `evaluation` goes between `code_intel` group and `evidence` per alphabetical order)
- Create: `crates/autospec-core/src/evaluation/mod.rs`, `error.rs`, `ids.rs`, `digest.rs`

**Interfaces:**
- Produces: `EvaluationError`, `EvaluationErrorKind::{Invariant, Immutable, Integrity, Io, Parse, FailClosed, AccessDenied}`; `EvaluatorSlot` (9 variants, `as_str`, `parse`, `ALL`); `EvaluatorVersionRef { slot, version }` with `Display` `"<slot>@<n>"`; `EpochId(u64)` with `Display` `"epoch-%06d"`; string ids `AnchorSuiteId`, `AnchorCaseId`, `ChallengerTrialId`, `PromotionId`, `EvaluationId` (validated `[a-z0-9][a-z0-9._-]{0,63}`); `Digest` with `of_bytes`, `of_parts`, `parse`, `as_str`, `short()`.

- [ ] **Step 1: Write failing unit tests** in `ids.rs` and `digest.rs` (inline `#[cfg(test)]`):

```rust
// ids.rs tests
#[test]
fn version_ref_round_trips_through_display_and_parse() {
    let v = EvaluatorVersionRef { slot: EvaluatorSlot::Architecture, version: 12 };
    assert_eq!(v.to_string(), "architecture@12");
    assert_eq!(EvaluatorVersionRef::parse("architecture@12").unwrap(), v);
    assert!(EvaluatorVersionRef::parse("architecture@0").is_err());
    assert!(EvaluatorVersionRef::parse("judge@1").is_err());
}
#[test]
fn epoch_id_formats_six_digits() {
    assert_eq!(EpochId(1).to_string(), "epoch-000001");
    assert_eq!(EpochId::parse("epoch-000042").unwrap(), EpochId(42));
    assert!(EpochId::parse("epoch-42").is_err());
}
#[test]
fn string_ids_reject_unsafe_characters() {
    assert!(AnchorCaseId::parse("case-01").is_ok());
    assert!(AnchorCaseId::parse("../case").is_err());
    assert!(AnchorCaseId::parse("Case").is_err());
    assert!(AnchorCaseId::parse("").is_err());
}
#[test]
fn serde_uses_string_form() {
    let json = serde_json::to_string(&EvaluatorVersionRef { slot: EvaluatorSlot::Docs, version: 3 }).unwrap();
    assert_eq!(json, "\"documentation@3\"");
    let json = serde_json::to_string(&EpochId(7)).unwrap();
    assert_eq!(json, "\"epoch-000007\"");
}
// digest.rs tests
#[test]
fn of_parts_is_nul_separated_sha256() {
    let a = Digest::of_parts(&[b"x", b"y"]);
    let b = Digest::of_bytes(b"x\0y");
    assert_eq!(a, b);
    assert_ne!(a, Digest::of_parts(&[b"xy"]));
    assert_eq!(a.as_str().len(), 64);
    assert_eq!(a.short().len(), 16);
}
#[test]
fn parse_requires_64_lowercase_hex() {
    assert!(Digest::parse(&"a".repeat(64)).is_ok());
    assert!(Digest::parse(&"A".repeat(64)).is_err());
    assert!(Digest::parse("abc").is_err());
}
```

- [ ] **Step 2: Run to verify failure:** `cargo test -p autospec-core --lib evaluation` → compile error (module missing).

- [ ] **Step 3: Implement.** `mod.rs`:

```rust
//! Versioned learned evaluators, frozen per-slot epochs, protected anchor
//! qualification, and controlled promotion.
//! Design: docs/specs/2026-09-05-evaluator-coevolution-design.md, ADR 0002.
pub mod anchor;
pub mod digest;
pub mod epoch;
pub mod error;
pub mod evaluator;
pub mod ids;
pub mod policy;
pub mod promotion;
pub mod qualification;
pub mod record;
pub mod statistics;
pub mod store;

/// Bumped when any persisted `.autospec/evaluation/**` document changes shape.
pub const EVALUATION_SCHEMA_VERSION: u64 = 1;

pub use error::{EvaluationError, EvaluationErrorKind};
```

(Until later tasks exist, declare only `digest`, `error`, `ids` and add the others as each task lands.)

`error.rs`:

```rust
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationErrorKind { Invariant, Immutable, Integrity, Io, Parse, FailClosed, AccessDenied }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationError { pub kind: EvaluationErrorKind, pub message: String }

impl EvaluationError {
    pub fn new(kind: EvaluationErrorKind, message: impl Into<String>) -> Self { Self { kind, message: message.into() } }
    pub fn invariant(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::Invariant, m) }
    pub fn immutable(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::Immutable, m) }
    pub fn integrity(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::Integrity, m) }
    pub fn io(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::Io, m) }
    pub fn parse(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::Parse, m) }
    pub fn fail_closed(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::FailClosed, m) }
    pub fn access_denied(m: impl Into<String>) -> Self { Self::new(EvaluationErrorKind::AccessDenied, m) }
}
impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            EvaluationErrorKind::Invariant => "invariant", EvaluationErrorKind::Immutable => "immutable",
            EvaluationErrorKind::Integrity => "integrity", EvaluationErrorKind::Io => "io",
            EvaluationErrorKind::Parse => "parse", EvaluationErrorKind::FailClosed => "fail-closed",
            EvaluationErrorKind::AccessDenied => "access-denied",
        };
        write!(f, "{kind}: {}", self.message)
    }
}
impl std::error::Error for EvaluationError {}
impl From<std::io::Error> for EvaluationError { fn from(e: std::io::Error) -> Self { Self::io(e.to_string()) } }
impl From<serde_json::Error> for EvaluationError { fn from(e: serde_json::Error) -> Self { Self::parse(e.to_string()) } }
```

`ids.rs`:

```rust
use std::fmt;
use serde::{Deserialize, Serialize};
use super::error::EvaluationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorSlot {
    SpecCompliance, Architecture, Maintainability, ComplexityDesign, TestQuality,
    SecurityReasoning, Documentation, UiUx, Operability,
}
impl EvaluatorSlot {
    pub const ALL: [EvaluatorSlot; 9] = [
        Self::SpecCompliance, Self::Architecture, Self::Maintainability, Self::ComplexityDesign,
        Self::TestQuality, Self::SecurityReasoning, Self::Documentation, Self::UiUx, Self::Operability,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpecCompliance => "spec_compliance", Self::Architecture => "architecture",
            Self::Maintainability => "maintainability", Self::ComplexityDesign => "complexity_design",
            Self::TestQuality => "test_quality", Self::SecurityReasoning => "security_reasoning",
            Self::Documentation => "documentation", Self::UiUx => "ui_ux", Self::Operability => "operability",
        }
    }
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        Self::ALL.iter().copied().find(|s| s.as_str() == value)
            .ok_or_else(|| EvaluationError::parse(format!("unknown evaluator slot {value:?}")))
    }
}
impl fmt::Display for EvaluatorSlot { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EvaluatorVersionRef { pub slot: EvaluatorSlot, pub version: u32 }
impl EvaluatorVersionRef {
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        let (slot, version) = value.split_once('@')
            .ok_or_else(|| EvaluationError::parse(format!("evaluator version must be <slot>@<n>, got {value:?}")))?;
        let version: u32 = version.parse().map_err(|_| EvaluationError::parse(format!("bad evaluator version number in {value:?}")))?;
        if version == 0 { return Err(EvaluationError::parse("evaluator versions start at 1")); }
        Ok(Self { slot: EvaluatorSlot::parse(slot)?, version })
    }
}
impl fmt::Display for EvaluatorVersionRef { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}@{}", self.slot, self.version) } }
impl TryFrom<String> for EvaluatorVersionRef { type Error = EvaluationError; fn try_from(v: String) -> Result<Self, Self::Error> { Self::parse(&v) } }
impl From<EvaluatorVersionRef> for String { fn from(v: EvaluatorVersionRef) -> String { v.to_string() } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EpochId(pub u64);
impl EpochId {
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        let digits = value.strip_prefix("epoch-").filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| EvaluationError::parse(format!("epoch id must be epoch-NNNNNN, got {value:?}")))?;
        Ok(Self(digits.parse().expect("six ascii digits parse")))
    }
    pub fn next(self) -> Self { Self(self.0 + 1) }
}
impl fmt::Display for EpochId { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "epoch-{:06}", self.0) } }
impl TryFrom<String> for EpochId { type Error = EvaluationError; fn try_from(v: String) -> Result<Self, Self::Error> { Self::parse(&v) } }
impl From<EpochId> for String { fn from(v: EpochId) -> String { v.to_string() } }

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
        && !value.contains("..")
}

macro_rules! string_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn parse(value: &str) -> Result<Self, EvaluationError> {
                if valid_id(value) { Ok(Self(value.to_string())) }
                else { Err(EvaluationError::parse(format!("invalid {}: {value:?} (expected [a-z0-9][a-z0-9._-]{{0,63}})", $label))) }
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl fmt::Display for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) } }
        impl TryFrom<String> for $name { type Error = EvaluationError; fn try_from(v: String) -> Result<Self, Self::Error> { Self::parse(&v) } }
        impl From<$name> for String { fn from(v: $name) -> String { v.0 } }
    };
}
string_id!(AnchorSuiteId, "anchor suite id");
string_id!(AnchorCaseId, "anchor case id");
string_id!(ChallengerTrialId, "challenger trial id");
string_id!(PromotionId, "promotion id");
string_id!(EvaluationId, "evaluation id");
```

`digest.rs`:

```rust
use std::fmt;
use serde::{Deserialize, Serialize};
use crate::autonomous::waterfall::sha256_hex;
use super::error::EvaluationError;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(String);
impl Digest {
    pub fn of_bytes(bytes: &[u8]) -> Self { Self(sha256_hex(bytes)) }
    /// NUL-separated canonical form, matching `review_evidence.rs` and the managed-project journal.
    pub fn of_parts(parts: &[&[u8]]) -> Self {
        let mut buf = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 { buf.push(0); }
            buf.extend_from_slice(part);
        }
        Self::of_bytes(&buf)
    }
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        if value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            Ok(Self(value.to_string()))
        } else {
            Err(EvaluationError::parse(format!("digest must be 64 lowercase hex chars, got {value:?}")))
        }
    }
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn short(&self) -> &str { &self.0[..16] }
}
impl fmt::Display for Digest { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) } }
impl TryFrom<String> for Digest { type Error = EvaluationError; fn try_from(v: String) -> Result<Self, Self::Error> { Self::parse(&v) } }
impl From<Digest> for String { fn from(v: Digest) -> String { v.0 } }
```

- [ ] **Step 4: Run tests:** `cargo test -p autospec-core --lib evaluation` → all pass. Also `cargo clippy -p autospec-core --all-targets`.

- [ ] **Step 5: Commit:** `git add crates/autospec-core/src/lib.rs crates/autospec-core/src/evaluation && git commit -m "feat(evaluation): module skeleton with typed ids, digests, and errors"`

---
### Task 3: Integer-only best-belief statistic

**Files:**
- Create: `crates/autospec-core/src/evaluation/statistics.rs`
- Create: `crates/autospec-core/tests/evaluation_statistics.rs`
- Modify: `crates/autospec-core/src/evaluation/mod.rs` (add `pub mod statistics;`)

**Interfaces:**
- Produces: `Ppm(pub u32)` (parts per million, `Ppm::ONE == Ppm(1_000_000)`, `Ppm::from_ratio(num: u32, den: u32) -> Option<Ppm>` rounding to nearest, `Ppm::as_u32`); `pub fn best_belief_ppm(successes: u32, failures: u32, epsilon: Ppm) -> Result<Ppm, EvaluationError>`; `pub const MAX_TRIALS: u32 = 65_536`.
- Consumes: `EvaluationError`.

Reference values (two independent methods agree to the ppm; see Global Constraints):

| S | F | BB at ε=0.05 (ppm) | BB at ε=0.10 (ppm) |
|---|---|---|---|
| 0 | 0 | 50 000 | 100 000 |
| 1 | 0 | 223 607 | 316 228 |
| 0 | 1 | 25 321 | 51 317 |
| 9 | 1 | 635 641 | 689 757 |
| 19 | 1 | 793 275 | 827 065 |
| 3 | 7 | 135 076 | 169 233 |
| 30 | 10 | 621 494 | 649 346 |
| 31 | 9 | 648 201 | 675 704 |
| 32 | 8 | 675 387 | 702 441 |
| 36 | 4 | 790 495 | 814 420 |
| 38 | 2 | 854 295 | 875 358 |
| 40 | 0 | 929 539 | 945 387 |
| 0 | 40 | 1 250 | 2 567 |
| 20 | 20 | 374 396 | 401 514 |
| 50 | 50 | 418 909 | 436 655 |
| 90 | 10 | 837 845 | 851 560 |
| 180 | 20 | 858 701 | 867 833 |

- [ ] **Step 1: Write the failing integration test** `crates/autospec-core/tests/evaluation_statistics.rs`:

```rust
use autospec_core::evaluation::statistics::{best_belief_ppm, Ppm, MAX_TRIALS};

const EPS_05: Ppm = Ppm(50_000);
const EPS_10: Ppm = Ppm(100_000);

// (successes, failures, expected ppm at eps=0.05, expected ppm at eps=0.10)
const REFERENCE: &[(u32, u32, u32, u32)] = &[
    (0, 0, 50_000, 100_000), (1, 0, 223_607, 316_228), (0, 1, 25_321, 51_317),
    (9, 1, 635_641, 689_757), (19, 1, 793_275, 827_065), (3, 7, 135_076, 169_233),
    (30, 10, 621_494, 649_346), (31, 9, 648_201, 675_704), (32, 8, 675_387, 702_441),
    (36, 4, 790_495, 814_420), (38, 2, 854_295, 875_358), (40, 0, 929_539, 945_387),
    (0, 40, 1_250, 2_567), (20, 20, 374_396, 401_514), (50, 50, 418_909, 436_655),
    (90, 10, 837_845, 851_560), (180, 20, 858_701, 867_833),
];

fn within(actual: Ppm, expected: u32, tolerance: u32) -> bool {
    actual.0.abs_diff(expected) <= tolerance
}

#[test]
fn best_belief_matches_independently_computed_beta_quantiles() {
    for &(s, f, e05, e10) in REFERENCE {
        let got05 = best_belief_ppm(s, f, EPS_05).unwrap();
        let got10 = best_belief_ppm(s, f, EPS_10).unwrap();
        assert!(within(got05, e05, 2), "S={s} F={f} eps=0.05: got {} want {e05}", got05.0);
        assert!(within(got10, e10, 2), "S={s} F={f} eps=0.10: got {} want {e10}", got10.0);
    }
}

#[test]
fn best_belief_is_monotone_in_successes_and_below_the_mean() {
    let mut previous = 0;
    for s in 0..=40 {
        let bb = best_belief_ppm(s, 40 - s, EPS_05).unwrap().0;
        assert!(bb >= previous, "S={s}: {bb} < {previous}");
        previous = bb;
        let mean_ppm = ((s as u64 + 1) * 1_000_000 / 42) as u32;
        assert!(bb <= mean_ppm, "S={s}: lower bound {bb} exceeds posterior mean {mean_ppm}");
    }
}

#[test]
fn best_belief_rejects_out_of_range_inputs() {
    assert!(best_belief_ppm(0, 0, Ppm(0)).is_err());
    assert!(best_belief_ppm(0, 0, Ppm(1_000_000)).is_err());
    assert!(best_belief_ppm(MAX_TRIALS, 1, EPS_05).is_err());
}

#[test]
fn ppm_from_ratio_rounds_to_nearest() {
    assert_eq!(Ppm::from_ratio(1, 3), Some(Ppm(333_333)));
    assert_eq!(Ppm::from_ratio(2, 3), Some(Ppm(666_667)));
    assert_eq!(Ppm::from_ratio(5, 5), Some(Ppm::ONE));
    assert_eq!(Ppm::from_ratio(1, 0), None);
}
```

- [ ] **Step 2: Run to verify failure:** `cargo test -p autospec-core --test evaluation_statistics` → compile error.

- [ ] **Step 3: Implement** `statistics.rs`:

```rust
//! Conservative binary qualification statistic (RQGM ε-best-belief) in integer
//! arithmetic. `BB_ε(S, F)` is the ε-quantile of `Beta(1+S, 1+F)`. For integer
//! parameters the Beta CDF is a binomial upper tail:
//! `I_x(a, b) = P[Bin(a+b-1, x) >= a]`, so the quantile is the smallest `x`
//! (in parts per million) whose tail probability reaches ε. No `f64`: the
//! `financial_no_f64` gate scans this crate.
use serde::{Deserialize, Serialize};
use super::error::EvaluationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ppm(pub u32);

impl Ppm {
    pub const ONE: Ppm = Ppm(1_000_000);
    pub fn from_ratio(numerator: u32, denominator: u32) -> Option<Ppm> {
        if denominator == 0 { return None; }
        let scaled = numerator as u64 * 1_000_000 + denominator as u64 / 2;
        Some(Ppm((scaled / denominator as u64).min(1_000_000) as u32))
    }
    pub fn as_u32(self) -> u32 { self.0 }
}

/// Largest `S + F + 1` supported; keeps every intermediate inside `u128`.
pub const MAX_TRIALS: u32 = 65_536;
/// Fixed-point scale for relative term weights (2^40).
const WEIGHT_ONE: u128 = 1u128 << 40;
/// Fixed-point scale for probabilities (2^64).
const PROB_ONE: u128 = 1u128 << 64;

/// `P[Bin(n, p) >= k]` scaled to `PROB_ONE`. Terms are computed relative to the
/// modal term so nothing under- or overflows for `n <= MAX_TRIALS`.
fn binomial_upper_tail(n: u32, k: u32, p: Ppm) -> u128 {
    if k == 0 { return PROB_ONE; }
    if k > n || p.0 == 0 { return 0; }
    if p.0 >= 1_000_000 { return PROB_ONE; }
    let p_num = p.0 as u128;
    let q_num = (1_000_000 - p.0) as u128;
    let mode = (((n as u128 + 1) * p_num) / 1_000_000).min(n as u128) as usize;
    let mut weights = vec![0u128; n as usize + 1];
    weights[mode] = WEIGHT_ONE;
    let mut j = mode;
    while j < n as usize {
        // t_{j+1} / t_j = (n - j) / (j + 1) * p / q
        let next = weights[j] * (n as u128 - j as u128) * p_num / ((j as u128 + 1) * q_num);
        weights[j + 1] = next;
        if next == 0 { break; }
        j += 1;
    }
    let mut j = mode;
    while j > 0 {
        // t_{j-1} / t_j = j / (n - j + 1) * q / p
        let prev = weights[j] * j as u128 * q_num / ((n as u128 - j as u128 + 1) * p_num);
        weights[j - 1] = prev;
        if prev == 0 { break; }
        j -= 1;
    }
    let total: u128 = weights.iter().sum();
    let tail: u128 = weights[k as usize..].iter().sum();
    (tail << 64) / total
}

/// ε-quantile of `Beta(1 + successes, 1 + failures)`, in parts per million.
pub fn best_belief_ppm(successes: u32, failures: u32, epsilon: Ppm) -> Result<Ppm, EvaluationError> {
    if epsilon.0 == 0 || epsilon.0 >= 1_000_000 {
        return Err(EvaluationError::invariant(format!("epsilon must be in (0, 1) ppm-exclusive, got {}", epsilon.0)));
    }
    let n = successes.checked_add(failures).and_then(|t| t.checked_add(1))
        .filter(|&t| t <= MAX_TRIALS)
        .ok_or_else(|| EvaluationError::invariant(format!("successes + failures + 1 must be <= {MAX_TRIALS}")))?;
    let k = successes + 1;
    // Parenthesised on purpose: `<<` binds looser than `/` in Rust.
    let threshold = ((epsilon.0 as u128) << 64) / 1_000_000;
    let (mut lo, mut hi) = (0u32, 1_000_000u32);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if binomial_upper_tail(n, k, Ppm(mid)) >= threshold { hi = mid; } else { lo = mid + 1; }
    }
    Ok(Ppm(lo))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_endpoints_are_exact() {
        assert_eq!(binomial_upper_tail(1, 1, Ppm(500_000)), PROB_ONE / 2);
        assert_eq!(binomial_upper_tail(5, 0, Ppm(1)), PROB_ONE);
        assert_eq!(binomial_upper_tail(5, 6, Ppm(999_999)), 0);
    }
    #[test]
    fn uniform_prior_quantile_is_epsilon_within_fixed_point_truncation() {
        assert!(best_belief_ppm(0, 0, Ppm(50_000)).unwrap().0.abs_diff(50_000) <= 1);
        assert!(best_belief_ppm(0, 0, Ppm(250_000)).unwrap().0.abs_diff(250_000) <= 1);
    }
}
```

- [ ] **Step 4: Run tests:** `cargo test -p autospec-core --test evaluation_statistics --lib evaluation::statistics` → pass. Then confirm the gate did not move: `bash scripts/architecture-fitness.sh run --registry .autospec/architecture-fitness.yml 2>&1 | grep financial_no_f64` → `observed=81`.

- [ ] **Step 5: Commit:** `git commit -am "feat(evaluation): integer-only epsilon-best-belief statistic with reference fixtures"` (add the new test file first).

---

### Task 4: Evaluator definitions

**Files:**
- Create: `crates/autospec-core/src/evaluation/evaluator.rs`
- Modify: `crates/autospec-core/src/evaluation/mod.rs` (`pub mod evaluator;`)

**Interfaces:**
- Produces: `EvaluatorKind::{Learned, Deterministic}`; `Provenance { created_by: String, source: String, notes: Option<String> }`; `EvaluatorDefinition { schema: u64, slot: EvaluatorSlot, version: u32, kind: EvaluatorKind, rubric_ref: String, prompt_digest: Option<Digest>, skill_pack_digest: Option<Digest>, knowledge_base_digest: Option<Digest>, runtime_settings_digest: Option<Digest>, routing_policy_digest: Digest, tool_policy_digest: Digest, model_family: Option<String>, created_at: u64, parent_version: Option<u32>, provenance: Provenance }` with `version_ref()`, `definition_digest()`, `validate()`.

- [ ] **Step 1: Write failing unit tests** (inline):

```rust
fn sample() -> EvaluatorDefinition {
    EvaluatorDefinition {
        schema: 1, slot: EvaluatorSlot::Architecture, version: 2, kind: EvaluatorKind::Learned,
        rubric_ref: "docs/rules/evaluator-qualification.rules.yaml#architecture".into(),
        prompt_digest: Some(Digest::of_bytes(b"prompt v2")), skill_pack_digest: None,
        knowledge_base_digest: None, runtime_settings_digest: None,
        routing_policy_digest: Digest::of_bytes(b"starter"), tool_policy_digest: Digest::of_bytes(b"read-only"),
        model_family: Some("anthropic".into()), created_at: 1_760_000_000, parent_version: Some(1),
        provenance: Provenance { created_by: "operator".into(), source: "manual".into(), notes: None },
    }
}
#[test]
fn definition_digest_ignores_timestamp_and_provenance_but_not_prompt() {
    let a = sample();
    let mut b = sample(); b.created_at += 1; b.provenance.created_by = "someone".into();
    assert_eq!(a.definition_digest(), b.definition_digest());
    let mut c = sample(); c.prompt_digest = Some(Digest::of_bytes(b"prompt v2 edited"));
    assert_ne!(a.definition_digest(), c.definition_digest());
}
#[test]
fn learned_evaluators_require_a_prompt_digest_and_versions_start_at_one() {
    let mut d = sample(); d.prompt_digest = None;
    assert_eq!(d.validate().unwrap_err().kind, EvaluationErrorKind::Invariant);
    let mut d = sample(); d.version = 0;
    assert!(d.validate().is_err());
    let mut d = sample(); d.parent_version = Some(2);
    assert!(d.validate().is_err(), "parent must be older than this version");
    let mut d = sample(); d.schema = 2;
    assert!(d.validate().is_err(), "future schema is rejected");
    assert!(sample().validate().is_ok());
}
#[test]
fn json_round_trip_preserves_every_field() {
    let d = sample();
    let text = serde_json::to_string_pretty(&d).unwrap();
    assert_eq!(serde_json::from_str::<EvaluatorDefinition>(&text).unwrap(), d);
}
```

- [ ] **Step 2: Run to verify failure:** `cargo test -p autospec-core --lib evaluation::evaluator` → compile error.

- [ ] **Step 3: Implement:**

```rust
use serde::{Deserialize, Serialize};
use super::digest::Digest;
use super::error::EvaluationError;
use super::ids::{EvaluatorSlot, EvaluatorVersionRef};
use super::EVALUATION_SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorKind { Learned, Deterministic }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub created_by: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorDefinition {
    pub schema: u64,
    pub slot: EvaluatorSlot,
    pub version: u32,
    pub kind: EvaluatorKind,
    pub rubric_ref: String,
    #[serde(default)] pub prompt_digest: Option<Digest>,
    #[serde(default)] pub skill_pack_digest: Option<Digest>,
    #[serde(default)] pub knowledge_base_digest: Option<Digest>,
    #[serde(default)] pub runtime_settings_digest: Option<Digest>,
    pub routing_policy_digest: Digest,
    pub tool_policy_digest: Digest,
    #[serde(default)] pub model_family: Option<String>,
    pub created_at: u64,
    #[serde(default)] pub parent_version: Option<u32>,
    pub provenance: Provenance,
}

fn opt(d: &Option<Digest>) -> &[u8] { d.as_ref().map(|d| d.as_str().as_bytes()).unwrap_or(b"") }

impl EvaluatorDefinition {
    pub fn version_ref(&self) -> EvaluatorVersionRef { EvaluatorVersionRef { slot: self.slot, version: self.version } }

    /// Behaviour-affecting fields only (handoff §5.4): never timestamp or provenance.
    pub fn definition_digest(&self) -> Digest {
        let kind = match self.kind { EvaluatorKind::Learned => "learned", EvaluatorKind::Deterministic => "deterministic" };
        let version = self.version.to_string();
        let parent = self.parent_version.map(|v| v.to_string()).unwrap_or_default();
        Digest::of_parts(&[
            self.slot.as_str().as_bytes(), version.as_bytes(), kind.as_bytes(), self.rubric_ref.as_bytes(),
            opt(&self.prompt_digest), opt(&self.skill_pack_digest), opt(&self.knowledge_base_digest),
            opt(&self.runtime_settings_digest), self.routing_policy_digest.as_str().as_bytes(),
            self.tool_policy_digest.as_str().as_bytes(),
            self.model_family.as_deref().unwrap_or("").as_bytes(), parent.as_bytes(),
        ])
    }

    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::invariant(format!("unsupported evaluator schema {}", self.schema)));
        }
        if self.version == 0 { return Err(EvaluationError::invariant("evaluator versions start at 1")); }
        if matches!(self.kind, EvaluatorKind::Learned) && self.prompt_digest.is_none() {
            return Err(EvaluationError::invariant("learned evaluators must pin a prompt_digest"));
        }
        if self.rubric_ref.trim().is_empty() { return Err(EvaluationError::invariant("rubric_ref is required")); }
        if let Some(parent) = self.parent_version {
            if parent >= self.version {
                return Err(EvaluationError::invariant(format!("parent_version {parent} must be older than version {}", self.version)));
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): immutable evaluator definitions with behaviour digests"`.

---

### Task 5: Anchor suites with protected holdout access

**Files:**
- Create: `crates/autospec-core/src/evaluation/anchor.rs`
- Modify: `mod.rs` (`pub mod anchor;`)

**Interfaces:**
- Produces: `ProtectedLabel::{Accept, Reject}`; `Severity::{Low, Medium, High, Critical}`; `AnchorVisibility::{Development, PublicRegression, ProtectedHoldout, Quarantine}`; `AccessRole::{Operator, Qualification, Mutation}`; `ProtectedSubset { name: String, tag: String, max_false_accept: Option<Ppm>, max_false_reject: Option<Ppm>, regression_tolerance_cases: u32 }`; `AnchorCase { case_id, artifact_ref: String, expected_label: Option<ProtectedLabel>, severity, tags: BTreeSet<String>, visibility, source: String, adjudication: Option<String>, content_digest: Digest }`; `AnchorSuite { schema, suite_id: AnchorSuiteId, version: u32, slot: EvaluatorSlot, cases: Vec<AnchorCase>, minimum_case_count: usize, required_subsets: Vec<ProtectedSubset>, provenance: Provenance }` with `validate()`, `suite_digest()`, `view(&self, AccessRole) -> AnchorSuite`, `verify_artifacts(&self, repo_root: &Path) -> Result<(), EvaluationError>`, `labeled_cases(&self) -> Result<Vec<(&AnchorCase, ProtectedLabel)>, EvaluationError>`.
- Consumes: `Ppm`, `Digest`, `Provenance`, ids.

- [ ] **Step 1: Write failing unit tests:**

```rust
fn case(id: &str, label: ProtectedLabel, vis: AnchorVisibility, tag: Option<&str>) -> AnchorCase {
    AnchorCase {
        case_id: AnchorCaseId::parse(id).unwrap(), artifact_ref: format!("fixtures/{id}/patch.diff"),
        expected_label: Some(label), severity: Severity::Medium,
        tags: tag.into_iter().map(String::from).collect(), visibility: vis,
        source: "synthetic".into(), adjudication: None, content_digest: Digest::of_bytes(id.as_bytes()),
    }
}
fn suite() -> AnchorSuite {
    AnchorSuite {
        schema: 1, suite_id: AnchorSuiteId::parse("architecture-fixture").unwrap(), version: 1,
        slot: EvaluatorSlot::Architecture,
        cases: vec![
            case("c1", ProtectedLabel::Accept, AnchorVisibility::Development, None),
            case("c2", ProtectedLabel::Reject, AnchorVisibility::ProtectedHoldout, Some("critical-security")),
            case("c3", ProtectedLabel::Reject, AnchorVisibility::Quarantine, None),
        ],
        minimum_case_count: 2,
        required_subsets: vec![ProtectedSubset { name: "critical".into(), tag: "critical-security".into(),
            max_false_accept: Some(Ppm(0)), max_false_reject: None, regression_tolerance_cases: 0 }],
        provenance: Provenance { created_by: "operator".into(), source: "fixture".into(), notes: None },
    }
}
#[test]
fn mutation_view_strips_holdout_labels_and_drops_quarantine() {
    let view = suite().view(AccessRole::Mutation);
    assert_eq!(view.cases.len(), 2);
    assert_eq!(view.cases[0].expected_label, Some(ProtectedLabel::Accept));
    assert_eq!(view.cases[1].expected_label, None);
    assert!(view.labeled_cases().is_err(), "a redacted view cannot qualify anything");
    assert_eq!(suite().view(AccessRole::Qualification).labeled_cases().unwrap().len(), 3);
}
#[test]
fn suite_digest_changes_when_a_label_flips() {
    let a = suite();
    let mut b = suite(); b.cases[0].expected_label = Some(ProtectedLabel::Reject);
    assert_ne!(a.suite_digest(), b.suite_digest());
    let mut c = suite(); c.provenance.notes = Some("irrelevant".into());
    assert_eq!(a.suite_digest(), c.suite_digest());
}
#[test]
fn validate_rejects_duplicates_missing_labels_and_unknown_subset_tags() {
    let mut s = suite(); s.cases.push(case("c1", ProtectedLabel::Accept, AnchorVisibility::Development, None));
    assert!(s.validate().is_err());
    let mut s = suite(); s.cases[0].expected_label = None;
    assert!(s.validate().is_err());
    let mut s = suite(); s.required_subsets[0].tag = "nope".into();
    assert!(s.validate().is_err());
    let mut s = suite(); s.minimum_case_count = 99;
    assert!(s.validate().is_err());
    let mut s = suite(); s.cases[0].artifact_ref = "/abs/path".into();
    assert!(s.validate().is_err());
    let mut s = suite(); s.cases[0].artifact_ref = "../escape".into();
    assert!(s.validate().is_err());
    assert!(suite().validate().is_ok());
}
#[test]
fn verify_artifacts_names_the_tampered_case() {
    let root = std::env::temp_dir().join(format!("autospec-anchor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let s = suite();
    for c in &s.cases {
        let path = root.join(&c.artifact_ref);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, c.case_id.as_str()).unwrap();
    }
    assert!(s.verify_artifacts(&root).is_ok());
    std::fs::write(root.join(&s.cases[1].artifact_ref), b"poisoned").unwrap();
    let err = s.verify_artifacts(&root).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Integrity);
    assert!(err.message.contains("c2"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p autospec-core --lib evaluation::anchor`.

- [ ] **Step 3: Implement** (key bodies; enums derive `Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize` with `rename_all = "snake_case"`):

```rust
impl AnchorSuite {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION { return Err(EvaluationError::invariant("unsupported anchor suite schema")); }
        if self.version == 0 { return Err(EvaluationError::invariant("anchor suite versions start at 1")); }
        let mut seen = BTreeSet::new();
        for case in &self.cases {
            if !seen.insert(&case.case_id) { return Err(EvaluationError::invariant(format!("duplicate anchor case {}", case.case_id))); }
            if case.expected_label.is_none() { return Err(EvaluationError::invariant(format!("anchor case {} has no expected_label", case.case_id))); }
            let path = Path::new(&case.artifact_ref);
            if path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
                return Err(EvaluationError::invariant(format!("anchor case {} artifact_ref must be repo-relative without '..'", case.case_id)));
            }
        }
        if self.minimum_case_count > self.cases.len() {
            return Err(EvaluationError::invariant(format!("minimum_case_count {} exceeds {} cases", self.minimum_case_count, self.cases.len())));
        }
        for subset in &self.required_subsets {
            if !self.cases.iter().any(|c| c.tags.contains(&subset.tag)) {
                return Err(EvaluationError::invariant(format!("required subset {} references tag {:?} that no case carries", subset.name, subset.tag)));
            }
        }
        Ok(())
    }

    /// Covers ids, labels, visibility, severity, tags, artifact digests, subsets, version. Not provenance.
    pub fn suite_digest(&self) -> Digest {
        let mut parts: Vec<Vec<u8>> = vec![self.suite_id.as_str().into(), self.version.to_string().into(), self.slot.as_str().into()];
        for case in &self.cases {
            let label = match case.expected_label { Some(ProtectedLabel::Accept) => "accept", Some(ProtectedLabel::Reject) => "reject", None => "" };
            parts.push(format!("{}|{}|{:?}|{:?}|{}|{}", case.case_id, label, case.visibility, case.severity,
                case.tags.iter().cloned().collect::<Vec<_>>().join(","), case.content_digest).into_bytes());
        }
        for subset in &self.required_subsets {
            parts.push(format!("{}|{}|{:?}|{:?}|{}", subset.name, subset.tag, subset.max_false_accept, subset.max_false_reject, subset.regression_tolerance_cases).into_bytes());
        }
        let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        Digest::of_parts(&refs)
    }

    /// Redacts protected labels for roles that must never see them (handoff §11.3, §12.1).
    pub fn view(&self, role: AccessRole) -> AnchorSuite {
        let mut view = self.clone();
        if role == AccessRole::Mutation {
            view.cases.retain(|c| c.visibility != AnchorVisibility::Quarantine);
            for case in &mut view.cases {
                if case.visibility == AnchorVisibility::ProtectedHoldout { case.expected_label = None; }
            }
        }
        view
    }

    pub fn labeled_cases(&self) -> Result<Vec<(&AnchorCase, ProtectedLabel)>, EvaluationError> {
        self.cases.iter().map(|c| c.expected_label.map(|l| (c, l))
            .ok_or_else(|| EvaluationError::access_denied(format!("case {} has a redacted label; qualification needs AccessRole::Qualification", c.case_id))))
            .collect()
    }

    pub fn verify_artifacts(&self, repo_root: &Path) -> Result<(), EvaluationError> {
        let mut mismatches = Vec::new();
        for case in &self.cases {
            let bytes = std::fs::read(repo_root.join(&case.artifact_ref))
                .map_err(|e| EvaluationError::integrity(format!("anchor case {} artifact {} unreadable: {e}", case.case_id, case.artifact_ref)))?;
            if Digest::of_bytes(&bytes) != case.content_digest { mismatches.push(case.case_id.to_string()); }
        }
        if mismatches.is_empty() { Ok(()) } else {
            Err(EvaluationError::integrity(format!("anchor artifact digest mismatch for cases: {}", mismatches.join(", "))))
        }
    }
}
```

- [ ] **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): anchor suites with protected holdout views and artifact verification"`.

---
### Task 6: Promotion policy

**Files:**
- Create: `crates/autospec-core/src/evaluation/policy.rs`
- Modify: `mod.rs` (`pub mod policy;`)

**Interfaces:**
- Produces: `PromotionPolicy { schema: u64, epsilon: Ppm, minimum_margin: Ppm, minimum_cases: usize, require_human_approval_slots: BTreeSet<EvaluatorSlot> }` with `Default` (ε = 50 000 ppm, margin = 10 000 ppm, minimum_cases = 40, human approval for `architecture` and `security_reasoning`), `validate()`, `policy_digest()`, `from_json(&str)`, `to_json()`.

Why 40: `docs/memory/feedback_quant_quality_measured_not_assumed.md` records that at n=40 a difference under ~7 items is noise; the margin check is on the lower bound, so the policy states its resolution limit rather than hiding it.

- [ ] **Step 1: Failing unit tests:**

```rust
#[test]
fn default_policy_is_conservative_and_digest_stable() {
    let p = PromotionPolicy::default();
    assert_eq!(p.epsilon, Ppm(50_000));
    assert_eq!(p.minimum_margin, Ppm(10_000));
    assert_eq!(p.minimum_cases, 40);
    assert!(p.require_human_approval_slots.contains(&EvaluatorSlot::Architecture));
    assert!(p.require_human_approval_slots.contains(&EvaluatorSlot::SecurityReasoning));
    assert_eq!(p.policy_digest(), PromotionPolicy::from_json(&p.to_json()).unwrap().policy_digest());
    let mut q = p.clone(); q.minimum_margin = Ppm(20_000);
    assert_ne!(p.policy_digest(), q.policy_digest());
}
#[test]
fn validate_bounds_epsilon_and_margin() {
    let mut p = PromotionPolicy::default(); p.epsilon = Ppm(0);
    assert!(p.validate().is_err());
    let mut p = PromotionPolicy::default(); p.epsilon = Ppm(500_001);
    assert!(p.validate().is_err(), "epsilon above one half is not a lower bound");
    let mut p = PromotionPolicy::default(); p.minimum_cases = 0;
    assert!(p.validate().is_err());
    assert!(PromotionPolicy::from_json(r#"{"schema":1,"epsilon":50000,"minimum_margin":10000,"minimum_cases":40,"require_human_approval_slots":["architecture"],"extra":1}"#).is_err(), "unknown fields are rejected");
}
```

- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement:**

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionPolicy {
    pub schema: u64,
    pub epsilon: Ppm,
    pub minimum_margin: Ppm,
    pub minimum_cases: usize,
    #[serde(default)] pub require_human_approval_slots: BTreeSet<EvaluatorSlot>,
}
impl Default for PromotionPolicy {
    fn default() -> Self {
        Self { schema: EVALUATION_SCHEMA_VERSION, epsilon: Ppm(50_000), minimum_margin: Ppm(10_000), minimum_cases: 40,
            require_human_approval_slots: [EvaluatorSlot::Architecture, EvaluatorSlot::SecurityReasoning].into_iter().collect() }
    }
}
impl PromotionPolicy {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION { return Err(EvaluationError::invariant("unsupported policy schema")); }
        if self.epsilon.0 == 0 || self.epsilon.0 > 500_000 { return Err(EvaluationError::invariant("epsilon must be in (0, 0.5]")); }
        if self.minimum_margin.0 > 500_000 { return Err(EvaluationError::invariant("minimum_margin must be <= 0.5")); }
        if self.minimum_cases == 0 { return Err(EvaluationError::invariant("minimum_cases must be >= 1")); }
        Ok(())
    }
    pub fn policy_digest(&self) -> Digest {
        let slots = self.require_human_approval_slots.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
        Digest::of_parts(&[self.schema.to_string().as_bytes(), self.epsilon.0.to_string().as_bytes(),
            self.minimum_margin.0.to_string().as_bytes(), self.minimum_cases.to_string().as_bytes(), slots.as_bytes()])
    }
    pub fn from_json(text: &str) -> Result<Self, EvaluationError> { let p: Self = serde_json::from_str(text)?; p.validate()?; Ok(p) }
    pub fn to_json(&self) -> String { serde_json::to_string_pretty(self).expect("policy serializes") }
}
```

- [ ] **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): promotion policy with digest and human-approval slots"`.

---

### Task 7: Paired qualification and the verdict seam

**Files:**
- Create: `crates/autospec-core/src/evaluation/qualification.rs`
- Create: `crates/autospec-core/tests/evaluation_qualification.rs`
- Create fixtures under `crates/autospec-core/tests/fixtures/evaluation/`: `anchors/architecture-fixture/v1.json`, `anchors/architecture-fixture/cases/case-01/patch.diff` … `case-40/patch.diff`, `verdicts/architecture-v1.json`, `verdicts/architecture-v2.json`, `verdicts/architecture-v2-regress.json`, `verdicts/architecture-v3-tie.json`, `verdicts/architecture-v2-incomplete.json`
- Modify: `mod.rs` (`pub mod qualification;`)

**Interfaces:**
- Produces: `Verdict::{Accept, Reject, Unavailable}`; `PairedCaseResult { case_id, expected: ProtectedLabel, severity, tags, incumbent: Verdict, challenger: Verdict }`; `pair_results(suite: &AnchorSuite, incumbent: &BTreeMap<AnchorCaseId, Verdict>, challenger: &BTreeMap<AnchorCaseId, Verdict>) -> Result<Vec<PairedCaseResult>, EvaluationError>`; `EvaluatorTally { successes, failures, unavailable, false_accepts, false_rejects: u32, best_belief: Ppm }`; `SubsetOutcome { name, cases, incumbent_errors, challenger_errors, challenger_false_accepts, challenger_false_rejects: u32, ceiling_breached: bool, regressed: bool }`; `Regression { subset: String, reason: String }`; `ChallengerVerdict::{Qualified, Rejected { reasons: Vec<String> }, Inconclusive { reasons: Vec<String> }}`; `QualificationReport { cases: usize, incumbent: EvaluatorTally, challenger: EvaluatorTally, challenger_only_correct: u32, incumbent_only_correct: u32, margin_ppm: i64, subsets: Vec<SubsetOutcome>, regressions: Vec<Regression>, verdict: ChallengerVerdict }`; `qualify(policy, suite, results) -> Result<QualificationReport, EvaluationError>`; `ChallengerTrial { schema, trial_id: ChallengerTrialId, slot, incumbent: EvaluatorVersionRef, challenger: EvaluatorVersionRef, suite_id, suite_version: u32, suite_digest: Digest, policy_digest: Digest, report: QualificationReport, results: Vec<PairedCaseResult>, created_at: u64 }` with `ChallengerTrial::id_for(slot, incumbent, challenger, suite_digest, results) -> ChallengerTrialId` (`ct-` + 16 hex of a digest over those parts); `trait VerdictSource { fn verdicts(&self, evaluator: EvaluatorVersionRef, suite: &AnchorSuite) -> Result<BTreeMap<AnchorCaseId, Verdict>, EvaluationError>; }`; `RecordedVerdicts { schema, evaluator: EvaluatorVersionRef, suite_id, suite_version, verdicts: BTreeMap<AnchorCaseId, Verdict>, runtime: RuntimeProvenance }` implementing `VerdictSource` (errors if evaluator or suite mismatch) with `from_json`.
- Consumes: Task 3 `best_belief_ppm`, Task 5 suite, Task 6 policy, Task 8 `RuntimeProvenance` (define `RuntimeProvenance` in Task 8 first if executing out of order; it is a plain struct — see Task 8).

Qualification rules, in order (D4):

1. `results.len() < policy.minimum_cases` → `Inconclusive["insufficient_cases: <n> < <min>"]`.
2. any `Unavailable` on either side → `Inconclusive["incomplete_verdicts: <k> cases"]` (handoff §22 "challenger result incomplete → no promotion"; orchestrator issue #39's `NO-OUTPUT` maps here).
3. tallies: success = verdict matches label; `best_belief = best_belief_ppm(S, F, policy.epsilon)`.
4. subsets: for each `required_subsets` entry, over cases carrying `tag`: `ceiling_breached` if challenger false-accept rate > `max_false_accept` or false-reject rate > `max_false_reject`; `regressed` if `challenger_errors > incumbent_errors + regression_tolerance_cases`. Any → `Rejected` with one reason per subset.
5. `margin = BB_ch − BB_inc`. `margin < 0` → `Rejected["lower_best_belief"]`. `margin < minimum_margin` (includes the tie) → `Inconclusive["margin_below_minimum: <margin> < <min>"]` (paper: ties favour the incumbent; AutoSpec: gather more evidence).
6. `challenger_only_correct <= incumbent_only_correct` → `Inconclusive["paired_advantage_absent"]` (pair structure, handoff §9.3).
7. otherwise `Qualified`.

Fixture design (40 cases, `case-01`…`case-40`; odd ids `accept`, even ids `reject`, except `case-31`…`case-35` which are all `reject` with tag `critical-security`, severity `critical`, visibility `protected_holdout`; `case-36`…`case-40` visibility `protected_holdout`; the rest `development`; `patch.diff` content is `--- a/<id>\n+++ b/<id>\n` and `content_digest` is its SHA-256; `minimum_case_count: 40`; one required subset `{ name: "critical", tag: "critical-security", max_false_accept: 0, regression_tolerance_cases: 0 }`):

| Verdict file | Wrong on | Tally | BB (ppm) | Expected |
|---|---|---|---|---|
| `architecture-v1.json` | 05, 10, 15, 20, 25, 30, 36, 37, 38, 39 | 30/10 | 621 494 | incumbent |
| `architecture-v2.json` | 05, 10, 36, 37 | 36/4 | 790 495 | `qualified` (margin 169 001; b=6, c=0) |
| `architecture-v2-regress.json` | 05, 10, 31, 32 | 36/4 | 790 495 | `rejected` (critical false accepts 2 > ceiling 0) |
| `architecture-v3-tie.json` | 03, 06, 08, 11, 16, 21, 26, 29, 38, 40 | 30/10 | 621 494 | `inconclusive` (margin 0; no critical case touched, so no subset regression) |
| `architecture-v2-incomplete.json` | as v2 but `case-40: unavailable` | — | — | `inconclusive` (incomplete) |

- [ ] **Step 1: Write the fixtures** with a small generator script run once (do not commit the script; commit its output): for each id write `patch.diff`, compute the digest with `sha256sum`, and assemble `v1.json`. Verdict files have this shape:

```json
{
  "schema": 1,
  "evaluator": "architecture@1",
  "suite_id": "architecture-fixture",
  "suite_version": 1,
  "runtime": { "model_id": "fixture-model", "provider_family": "fixture", "execution_identity": "recorded", "independence": "high" },
  "verdicts": { "case-01": "accept", "case-02": "reject", "...": "..." }
}
```

- [ ] **Step 2: Write the failing integration test** `crates/autospec-core/tests/evaluation_qualification.rs`:

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use autospec_core::evaluation::anchor::{AccessRole, AnchorSuite};
use autospec_core::evaluation::ids::EvaluatorVersionRef;
use autospec_core::evaluation::policy::PromotionPolicy;
use autospec_core::evaluation::qualification::{pair_results, qualify, ChallengerVerdict, RecordedVerdicts, VerdictSource};
use autospec_core::evaluation::statistics::Ppm;

fn fixtures() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evaluation") }
fn suite() -> AnchorSuite {
    let text = std::fs::read_to_string(fixtures().join("anchors/architecture-fixture/v1.json")).unwrap();
    let suite: AnchorSuite = serde_json::from_str(&text).unwrap();
    suite.validate().unwrap();
    suite
}
fn verdicts(name: &str) -> RecordedVerdicts {
    RecordedVerdicts::from_json(&std::fs::read_to_string(fixtures().join(format!("verdicts/{name}.json"))).unwrap()).unwrap()
}
fn report(challenger_file: &str) -> autospec_core::evaluation::qualification::QualificationReport {
    let suite = suite();
    let inc = verdicts("architecture-v1").verdicts(EvaluatorVersionRef::parse("architecture@1").unwrap(), &suite).unwrap();
    let ch_file = verdicts(challenger_file);
    let ch = ch_file.verdicts(ch_file.evaluator, &suite).unwrap();
    let results = pair_results(&suite.view(AccessRole::Qualification), &inc, &ch).unwrap();
    qualify(&PromotionPolicy::default(), &suite, &results).unwrap()
}

#[test]
fn fixture_artifacts_match_their_digests() { suite().verify_artifacts(&fixtures().join("anchors/architecture-fixture")).unwrap(); }

#[test]
fn v2_qualifies_with_paired_advantage() {
    let r = report("architecture-v2");
    assert_eq!(r.incumbent.best_belief, Ppm(621_494));
    assert_eq!(r.challenger.best_belief, Ppm(790_495));
    assert_eq!((r.challenger_only_correct, r.incumbent_only_correct), (6, 0));
    assert_eq!(r.margin_ppm, 169_001);
    assert_eq!(r.verdict, ChallengerVerdict::Qualified);
}
#[test]
fn v2_regress_is_rejected_on_the_critical_subset() {
    let r = report("architecture-v2-regress");
    assert_eq!(r.subsets[0].challenger_false_accepts, 2);
    assert!(r.subsets[0].ceiling_breached && r.subsets[0].regressed);
    assert!(matches!(r.verdict, ChallengerVerdict::Rejected { .. }), "{:?}", r.verdict);
}
#[test]
fn v3_tie_is_inconclusive_not_promoted() {
    let r = report("architecture-v3-tie");
    assert_eq!(r.margin_ppm, 0);
    assert!(matches!(&r.verdict, ChallengerVerdict::Inconclusive { reasons } if reasons[0].starts_with("margin_below_minimum")), "{:?}", r.verdict);
}
#[test]
fn incomplete_verdicts_are_inconclusive() {
    let r = report("architecture-v2-incomplete");
    assert!(matches!(&r.verdict, ChallengerVerdict::Inconclusive { reasons } if reasons[0].starts_with("incomplete_verdicts")));
}
#[test]
fn a_redacted_suite_cannot_be_paired() {
    let suite = suite().view(AccessRole::Mutation);
    let empty = BTreeMap::new();
    assert!(pair_results(&suite, &empty, &empty).is_err());
}
#[test]
fn recorded_verdicts_refuse_a_mismatched_evaluator() {
    let v = verdicts("architecture-v2");
    assert!(v.verdicts(EvaluatorVersionRef::parse("architecture@9").unwrap(), &suite()).is_err());
}
#[test]
fn too_few_cases_is_inconclusive() {
    let mut suite = suite();
    suite.cases.truncate(10); suite.minimum_case_count = 10;
    let inc = verdicts("architecture-v1"); let ch = verdicts("architecture-v2");
    let results = pair_results(&suite, &inc.verdicts(inc.evaluator, &suite).unwrap(), &ch.verdicts(ch.evaluator, &suite).unwrap()).unwrap();
    let r = qualify(&PromotionPolicy::default(), &suite, &results).unwrap();
    assert!(matches!(&r.verdict, ChallengerVerdict::Inconclusive { reasons } if reasons[0].starts_with("insufficient_cases")));
}
```

- [ ] **Step 3: Run to verify failure.** `cargo test -p autospec-core --test evaluation_qualification`.

- [ ] **Step 4: Implement `qualification.rs`** (core of `qualify`; the rest is plain structs with serde `snake_case` enums):

```rust
pub fn pair_results(suite: &AnchorSuite, incumbent: &BTreeMap<AnchorCaseId, Verdict>, challenger: &BTreeMap<AnchorCaseId, Verdict>)
    -> Result<Vec<PairedCaseResult>, EvaluationError> {
    suite.labeled_cases()?.into_iter().map(|(case, expected)| Ok(PairedCaseResult {
        case_id: case.case_id.clone(), expected, severity: case.severity, tags: case.tags.clone(),
        incumbent: incumbent.get(&case.case_id).copied().unwrap_or(Verdict::Unavailable),
        challenger: challenger.get(&case.case_id).copied().unwrap_or(Verdict::Unavailable),
    })).collect()
}

fn tally(results: &[PairedCaseResult], pick: fn(&PairedCaseResult) -> Verdict, epsilon: Ppm) -> Result<EvaluatorTally, EvaluationError> {
    let mut t = EvaluatorTally { successes: 0, failures: 0, unavailable: 0, false_accepts: 0, false_rejects: 0, best_belief: Ppm(0) };
    for r in results {
        match (pick(r), r.expected) {
            (Verdict::Unavailable, _) => t.unavailable += 1,
            (Verdict::Accept, ProtectedLabel::Accept) | (Verdict::Reject, ProtectedLabel::Reject) => t.successes += 1,
            (Verdict::Accept, ProtectedLabel::Reject) => { t.failures += 1; t.false_accepts += 1; }
            (Verdict::Reject, ProtectedLabel::Accept) => { t.failures += 1; t.false_rejects += 1; }
        }
    }
    t.best_belief = best_belief_ppm(t.successes, t.failures, epsilon)?;
    Ok(t)
}

pub fn qualify(policy: &PromotionPolicy, suite: &AnchorSuite, results: &[PairedCaseResult]) -> Result<QualificationReport, EvaluationError> {
    policy.validate()?;
    let incumbent = tally(results, |r| r.incumbent, policy.epsilon)?;
    let challenger = tally(results, |r| r.challenger, policy.epsilon)?;
    let mut report = QualificationReport { cases: results.len(), incumbent, challenger, challenger_only_correct: 0, incumbent_only_correct: 0,
        margin_ppm: 0, subsets: Vec::new(), regressions: Vec::new(), verdict: ChallengerVerdict::Qualified };
    if results.len() < policy.minimum_cases {
        report.verdict = ChallengerVerdict::Inconclusive { reasons: vec![format!("insufficient_cases: {} < {}", results.len(), policy.minimum_cases)] };
        return Ok(report);
    }
    let unavailable = report.incumbent.unavailable + report.challenger.unavailable;
    if unavailable > 0 {
        report.verdict = ChallengerVerdict::Inconclusive { reasons: vec![format!("incomplete_verdicts: {unavailable} cases")] };
        return Ok(report);
    }
    for r in results {
        let inc_ok = r.incumbent.matches(r.expected) == Some(true);
        let ch_ok = r.challenger.matches(r.expected) == Some(true);
        if ch_ok && !inc_ok { report.challenger_only_correct += 1; }
        if inc_ok && !ch_ok { report.incumbent_only_correct += 1; }
    }
    for subset in &suite.required_subsets {
        let members: Vec<&PairedCaseResult> = results.iter().filter(|r| r.tags.contains(&subset.tag)).collect();
        let mut o = SubsetOutcome { name: subset.name.clone(), cases: members.len() as u32, incumbent_errors: 0, challenger_errors: 0,
            challenger_false_accepts: 0, challenger_false_rejects: 0, ceiling_breached: false, regressed: false };
        let (mut expected_rejects, mut expected_accepts) = (0u32, 0u32);
        for r in &members {
            match r.expected { ProtectedLabel::Reject => expected_rejects += 1, ProtectedLabel::Accept => expected_accepts += 1 }
            if r.incumbent.matches(r.expected) == Some(false) { o.incumbent_errors += 1; }
            match (r.challenger, r.expected) {
                (Verdict::Accept, ProtectedLabel::Reject) => { o.challenger_errors += 1; o.challenger_false_accepts += 1; }
                (Verdict::Reject, ProtectedLabel::Accept) => { o.challenger_errors += 1; o.challenger_false_rejects += 1; }
                _ => {}
            }
        }
        if let (Some(ceiling), Some(rate)) = (subset.max_false_accept, Ppm::from_ratio(o.challenger_false_accepts, expected_rejects)) { if rate > ceiling { o.ceiling_breached = true; } }
        if let (Some(ceiling), Some(rate)) = (subset.max_false_reject, Ppm::from_ratio(o.challenger_false_rejects, expected_accepts)) { if rate > ceiling { o.ceiling_breached = true; } }
        if o.challenger_errors > o.incumbent_errors + subset.regression_tolerance_cases { o.regressed = true; }
        if o.ceiling_breached { report.regressions.push(Regression { subset: subset.name.clone(), reason: format!("ceiling_breached: {} false accepts / {} false rejects", o.challenger_false_accepts, o.challenger_false_rejects) }); }
        if o.regressed { report.regressions.push(Regression { subset: subset.name.clone(), reason: format!("regressed_vs_incumbent: {} > {} + {}", o.challenger_errors, o.incumbent_errors, subset.regression_tolerance_cases) }); }
        report.subsets.push(o);
    }
    report.margin_ppm = report.challenger.best_belief.0 as i64 - report.incumbent.best_belief.0 as i64;
    if !report.regressions.is_empty() {
        report.verdict = ChallengerVerdict::Rejected { reasons: report.regressions.iter().map(|r| format!("{}: {}", r.subset, r.reason)).collect() };
    } else if report.margin_ppm < 0 {
        report.verdict = ChallengerVerdict::Rejected { reasons: vec!["lower_best_belief".into()] };
    } else if report.margin_ppm < policy.minimum_margin.0 as i64 {
        report.verdict = ChallengerVerdict::Inconclusive { reasons: vec![format!("margin_below_minimum: {} < {}", report.margin_ppm, policy.minimum_margin.0)] };
    } else if report.challenger_only_correct <= report.incumbent_only_correct {
        report.verdict = ChallengerVerdict::Inconclusive { reasons: vec!["paired_advantage_absent".into()] };
    }
    Ok(report)
}
```

`ChallengerTrial::id_for` = `ChallengerTrialId::parse(&format!("ct-{}", Digest::of_parts(&[slot, inc, ch, suite_digest, serde_json(results)]).short()))`.

- [ ] **Step 5: Run tests:** pass. **Step 6: Commit:** `git add crates/autospec-core/tests/fixtures/evaluation crates/autospec-core/tests/evaluation_qualification.rs crates/autospec-core/src/evaluation && git commit -m "feat(evaluation): paired challenger qualification with recorded-verdict seam and fixtures"`.

---

### Task 8: Epochs, evaluation records, and promotion planning

**Files:**
- Create: `crates/autospec-core/src/evaluation/epoch.rs`, `record.rs`, `promotion.rs`
- Modify: `mod.rs`

**Interfaces:**
- Produces (`epoch.rs`): `EvaluatorEpoch { schema, epoch_id: EpochId, slot_versions: BTreeMap<EvaluatorSlot, u32>, started_at: u64, predecessor: Option<EpochId>, promotion: Option<PromotionId>, policy_digest: Digest, anchor_suite_digests: BTreeMap<AnchorSuiteId, Digest> }`; `EvaluatorEpoch::genesis(policy_digest, at)`; `version_of(slot) -> Option<u32>`; `successor(&self, slot, version, promotion, policy_digest, suite: Option<(AnchorSuiteId, Digest)>, at) -> Result<EvaluatorEpoch, EvaluationError>` (id = `self.epoch_id.next()`; errors unless `version > existing`).
- Produces (`record.rs`): `Independence::{High, Reduced, None}`; `RuntimeProvenance { model_id, provider_family, execution_identity: String, independence: Independence }`; `ActiveRankingStatus::{Active, StaleForActiveRanking { superseded_by: EvaluatorVersionRef, epoch: EpochId }, Replayed { replay_of: EvaluationId }, HistoricalOnly}`; `RankingTransition { at, from, to, promotion: Option<PromotionId> }`; `EvaluationRecord { schema, evaluation_id, subject: String, epoch_id, evaluator: EvaluatorVersionRef, outcome: Verdict, score: Option<Ppm>, evidence_refs: Vec<String>, runtime: RuntimeProvenance, created_at, active_ranking_status, ranking_history: Vec<RankingTransition> }` with `depends_on(slot, version) -> bool` (Active and exact version match) and `mark_stale(superseded_by, epoch, promotion, at) -> bool` (idempotent; returns whether it changed); `stale_candidates<'a>(records, slot, version) -> Vec<EvaluationId>` (sorted).
- Produces (`promotion.rs`): `ApprovalKind::{Policy, Human}`; `Approval { kind, actor: String, at: u64 }`; `PromotionState::{Pending, Committed}`; `PromotionEvent { schema, promotion_id, state, slot, from: Option<u32>, to: u32, challenger_trial: Option<ChallengerTrialId>, prior_epoch, new_epoch, invalidated_evaluations: Vec<EvaluationId>, approval, created_at, committed_at: Option<u64> }`; `plan_promotion(policy, current: &EvaluatorEpoch, trial: &ChallengerTrial, invalidated: Vec<EvaluationId>, approval, at) -> Result<PromotionEvent, EvaluationError>`; `plan_pin(policy, current, evaluator: EvaluatorVersionRef, approval, at) -> Result<PromotionEvent, EvaluationError>`; `PromotionId` = `promo-` + 16 hex of digest over (trial or evaluator, prior epoch, new epoch).

`plan_promotion` fails closed (`EvaluationErrorKind::FailClosed`) when: trial verdict ≠ `Qualified`; `trial.incumbent.version != current.version_of(slot)`; `trial.policy_digest != policy.policy_digest()`; `trial.challenger.version <= incumbent`; slot ∈ `require_human_approval_slots` and `approval.kind != Human`. `plan_pin` fails closed when the slot already has a version (pins seed only; replacements need a trial).

- [ ] **Step 1: Failing unit tests** (inline in each file; the promotion tests build a `ChallengerTrial` by hand with `verdict: Qualified`, `policy_digest: PromotionPolicy::default().policy_digest()`):

```rust
// epoch.rs
#[test]
fn successor_increments_id_and_requires_a_newer_version() {
    let g = EvaluatorEpoch::genesis(Digest::of_bytes(b"p"), 1);
    assert_eq!(g.epoch_id, EpochId(0));
    let e1 = g.successor(EvaluatorSlot::Architecture, 1, PromotionId::parse("promo-1").unwrap(), Digest::of_bytes(b"p"), None, 2).unwrap();
    assert_eq!(e1.epoch_id, EpochId(1));
    assert_eq!(e1.predecessor, Some(EpochId(0)));
    assert_eq!(e1.version_of(EvaluatorSlot::Architecture), Some(1));
    assert!(e1.successor(EvaluatorSlot::Architecture, 1, PromotionId::parse("promo-2").unwrap(), Digest::of_bytes(b"p"), None, 3).is_err());
    let e2 = e1.successor(EvaluatorSlot::Documentation, 1, PromotionId::parse("promo-3").unwrap(), Digest::of_bytes(b"p"), None, 3).unwrap();
    assert_eq!(e2.slot_versions.len(), 2, "other slots are carried forward");
}
// record.rs
#[test]
fn mark_stale_is_idempotent_and_keeps_history() {
    let mut r = sample_record(EvaluatorVersionRef::parse("architecture@1").unwrap());
    let to = EvaluatorVersionRef::parse("architecture@2").unwrap();
    assert!(r.depends_on(EvaluatorSlot::Architecture, 1));
    assert!(r.mark_stale(to, EpochId(2), PromotionId::parse("promo-x").unwrap(), 10));
    assert!(!r.mark_stale(to, EpochId(2), PromotionId::parse("promo-x").unwrap(), 11));
    assert_eq!(r.ranking_history.len(), 1);
    assert!(!r.depends_on(EvaluatorSlot::Architecture, 1));
    assert_eq!(r.outcome, Verdict::Accept, "judgment is untouched");
}
#[test]
fn stale_candidates_selects_only_the_displaced_version() {
    let records = vec![sample_record(EvaluatorVersionRef::parse("architecture@1").unwrap()),
        sample_record(EvaluatorVersionRef::parse("architecture@2").unwrap()),
        sample_record(EvaluatorVersionRef::parse("documentation@1").unwrap())];
    assert_eq!(stale_candidates(records.iter(), EvaluatorSlot::Architecture, 1).len(), 1);
}
// promotion.rs
#[test]
fn plan_promotion_fails_closed_without_human_approval_for_protected_slots() {
    let err = plan_promotion(&policy(), &epoch_with_arch_v1(), &qualified_trial(), vec![], policy_approval(), 5).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
    let ok = plan_promotion(&policy(), &epoch_with_arch_v1(), &qualified_trial(), vec![], human_approval(), 5).unwrap();
    assert_eq!((ok.from, ok.to, ok.state), (Some(1), 2, PromotionState::Pending));
    assert_eq!(ok.new_epoch, EpochId(2));
}
#[test]
fn plan_promotion_rejects_inconclusive_stale_incumbent_and_policy_drift() {
    let mut t = qualified_trial(); t.report.verdict = ChallengerVerdict::Inconclusive { reasons: vec![] };
    assert!(plan_promotion(&policy(), &epoch_with_arch_v1(), &t, vec![], human_approval(), 5).is_err());
    let mut t = qualified_trial(); t.incumbent.version = 7;
    assert!(plan_promotion(&policy(), &epoch_with_arch_v1(), &t, vec![], human_approval(), 5).is_err());
    let mut t = qualified_trial(); t.policy_digest = Digest::of_bytes(b"other");
    assert!(plan_promotion(&policy(), &epoch_with_arch_v1(), &t, vec![], human_approval(), 5).is_err());
}
#[test]
fn plan_pin_only_seeds_empty_slots() {
    assert!(plan_pin(&policy(), &epoch_with_arch_v1(), EvaluatorVersionRef::parse("architecture@2").unwrap(), human_approval(), 5).is_err());
    let pin = plan_pin(&policy(), &epoch_with_arch_v1(), EvaluatorVersionRef::parse("documentation@1").unwrap(), policy_approval(), 5).unwrap();
    assert_eq!((pin.from, pin.to, pin.challenger_trial), (None, 1, None));
}
```

- [ ] **Step 2: Run to verify failure.** **Step 3: Implement** per the interface list (straightforward structs; `successor` clones `slot_versions`, inserts, sets `predecessor: Some(self.epoch_id)`, `promotion: Some(promotion)`, `started_at: at`, merges `anchor_suite_digests`). **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): epochs, evaluation records with stale marking, and fail-closed promotion planning"`.

---
### Task 9: File store — layout, crash-safe I/O, immutable registry

**Files:**
- Create: `crates/autospec-core/src/evaluation/store/mod.rs`, `store/layout.rs`, `store/io.rs`
- Create: `crates/autospec-core/tests/evaluation_registry.rs`
- Modify: `evaluation/mod.rs` (`pub mod store;`)

**Interfaces:**
- Produces (`layout.rs`): `EvaluationLayout::new(repo_root: &Path)` with `root()` (= `<repo>/.autospec/evaluation`), `policy_file()`, `evaluator_file(slot, version)`, `evaluators_dir()`, `anchor_file(&AnchorSuiteId, version)`, `anchors_dir()`, `epoch_file(EpochId)`, `epochs_dir()`, `current_file()`, `trial_file(&ChallengerTrialId)`, `trials_dir()`, `promotion_file(&PromotionId)`, `promotions_dir()`, `record_file(&EvaluationId)`, `records_dir()`, `journal_file()`, `checkpoint_file()`, `ensure_directories() -> Result<(), EvaluationError>`.
- Produces (`io.rs`): `atomic_write(path, bytes) -> Result<(), EvaluationError>` (tmp `path.tmp-<pid>-<serial>` with `create_new`, `write_all`, `sync_all`, `rename`, parent-dir `sync_all` — the `managed_project.rs:259` body, minus the private-permission checks, since this state lives in the repo tree); `append_synced_line(path, line: &[u8], fail_after: Option<usize>) -> Result<(), EvaluationError>` (the `managed_project.rs:228` body: record length, seek end, write or injected partial write, rollback by `set_len` + `sync_all`); `write_immutable_json<T: Serialize>(path, value) -> Result<(), EvaluationError>` (`OpenOptions::create_new(true)`; `AlreadyExists` → `EvaluationErrorKind::Immutable` naming the path); `read_json<T: DeserializeOwned>(path) -> Result<T, EvaluationError>`; `read_json_if_exists<T>(path) -> Result<Option<T>, EvaluationError>`.
- Produces (`store/mod.rs`): `CurrentPointer { schema, epoch_id }`; `EvaluationStore { layout, policy }` with `init(repo_root, policy, at) -> Result<Self>` (fails `Immutable` if `policy.json` exists; writes policy, `epoch-000000` genesis, `current.json`, empty journal checkpoint), `open(repo_root) -> Result<Self>` (requires `policy.json` and `current.json`; verifies the journal chain; runs `recover_pending_promotions()` from Task 11), `policy()`, `layout()`, `register_evaluator(&EvaluatorDefinition) -> Result<Digest>` (validate, immutable write, journal `evaluator.version.created`), `evaluator(EvaluatorVersionRef) -> Result<EvaluatorDefinition>`, `list_evaluators() -> Result<Vec<EvaluatorDefinition>>` (sorted by slot, version), `register_anchor_suite(&AnchorSuite, repo_root) -> Result<Digest>` (validate, `verify_artifacts`, immutable write, journal `anchor.suite.verified`), `anchor_suite(&AnchorSuiteId, version, AccessRole) -> Result<AnchorSuite>` (applies `view(role)`), `list_anchor_suites() -> Result<Vec<(AnchorSuiteId, u32, Digest)>>`, `current_epoch() -> Result<EvaluatorEpoch>`, `epoch(EpochId) -> Result<EvaluatorEpoch>`, `epoch_history() -> Result<Vec<EvaluatorEpoch>>`, `write_trial(&ChallengerTrial) -> Result<()>` (immutable; journal `evaluator.challenger.completed`), `trial(&ChallengerTrialId) -> Result<ChallengerTrial>`, `write_record(&EvaluationRecord) -> Result<()>` (immutable on first write; journal `evaluation.completed`), `record(&EvaluationId) -> Result<EvaluationRecord>`, `records() -> Result<Vec<EvaluationRecord>>`, `promotion(&PromotionId) -> Result<PromotionEvent>`, `promotions() -> Result<Vec<PromotionEvent>>`.
- Journal calls in this task compile against Task 10's `Journal`; implement Tasks 9 and 10 in the same commit if executing serially, or stub journal calls behind `#[allow(unused)]` until Task 10 lands.

- [ ] **Step 1: Failing integration test** `crates/autospec-core/tests/evaluation_registry.rs` (temp-root helper copied from `evidence_bundle.rs`):

```rust
#[test]
fn init_is_once_and_open_requires_policy_and_pointer() {
    let root = TempProjectRoot::new();
    let store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    assert_eq!(store.current_epoch().unwrap().epoch_id, EpochId(0));
    assert_eq!(EvaluationStore::init(root.path(), PromotionPolicy::default(), 101).unwrap_err().kind, EvaluationErrorKind::Immutable);
    assert!(EvaluationStore::open(root.path()).is_ok());
    std::fs::remove_file(root.path().join(".autospec/evaluation/current.json")).unwrap();
    assert!(EvaluationStore::open(root.path()).is_err());
}
#[test]
fn evaluator_versions_cannot_be_rewritten() {
    let root = TempProjectRoot::new();
    let store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let def = fixture_definition(1);
    let digest = store.register_evaluator(&def).unwrap();
    assert_eq!(digest, def.definition_digest());
    let mut edited = def.clone(); edited.prompt_digest = Some(Digest::of_bytes(b"edited in place"));
    let err = store.register_evaluator(&edited).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Immutable);
    assert!(err.message.contains("evaluators/architecture/v1.json"), "{err}");
    assert_eq!(store.evaluator(def.version_ref()).unwrap(), def);
    assert_eq!(store.list_evaluators().unwrap().len(), 1);
}
#[test]
fn anchor_registration_verifies_artifacts_and_redacts_for_mutation_role() {
    let root = TempProjectRoot::new();
    copy_fixture_suite_into(root.path()); // copies tests/fixtures/evaluation/anchors/architecture-fixture/cases/** to <root>/fixtures/anchors/architecture-fixture/** and rewrites artifact_ref prefixes accordingly
    let store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let suite = fixture_suite_relocated();
    store.register_anchor_suite(&suite, root.path()).unwrap();
    let redacted = store.anchor_suite(&suite.suite_id, 1, AccessRole::Mutation).unwrap();
    assert!(redacted.cases.iter().filter(|c| c.visibility == AnchorVisibility::ProtectedHoldout).all(|c| c.expected_label.is_none()));
    let full = store.anchor_suite(&suite.suite_id, 1, AccessRole::Qualification).unwrap();
    assert_eq!(full.labeled_cases().unwrap().len(), 40);
    std::fs::write(root.path().join(&suite.cases[3].artifact_ref), b"tampered").unwrap();
    let mut v2 = suite.clone(); v2.version = 2;
    assert_eq!(store.register_anchor_suite(&v2, root.path()).unwrap_err().kind, EvaluationErrorKind::Integrity);
}
#[test]
fn records_and_trials_are_write_once() {
    let root = TempProjectRoot::new();
    let store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let record = fixture_record("ev-1", "architecture@1");
    store.write_record(&record).unwrap();
    assert_eq!(store.write_record(&record).unwrap_err().kind, EvaluationErrorKind::Immutable);
    assert_eq!(store.records().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run to verify failure.** **Step 3: Implement** `layout.rs`, `io.rs`, `store/mod.rs` per interfaces. `current.json` is written with `atomic_write`; every immutable document with `write_immutable_json`. `open()` verifies `policy.validate()` and the journal (Task 10) before returning. **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): repo-local immutable store with crash-safe writes"`.

---

### Task 10: Hash-chained journal with idempotency keys

**Files:**
- Create: `crates/autospec-core/src/evaluation/store/journal.rs`
- Modify: `store/mod.rs` (`pub mod journal;`, `EvaluationStore` holds `journal: Journal`)

**Interfaces:**
- Produces: `JournalEvent { schema: u64, sequence: u64, at: u64, kind: String, key: String, fields: BTreeMap<String, String> }`; `Checkpoint { schema, high_watermark: u64, digest: Digest }`; `Journal::open(layout: &EvaluationLayout) -> Result<Journal, EvaluationError>` (replays `events.jsonl`, recomputes the chain digest with `extend(prior, line) = sha256(prior || 0x00 || line)` starting from `sha256("autospec-evaluation-journal-v1")`, and compares against `events.checkpoint.json`; `Integrity` error if the journal is behind the checkpoint or the digest differs); `Journal::append(&mut self, at, kind, key, fields) -> Result<bool, EvaluationError>` (returns `false` and does nothing if `key` already exists with identical `kind`+`fields`; `Integrity` error if it exists with different content; otherwise assigns `sequence = high_watermark + 1`, `append_synced_line`, updates the in-memory digest, `atomic_write`s the checkpoint); `contains(&self, key) -> bool`; `events(&self) -> &[JournalEvent]`; `high_watermark()`; `#[doc(hidden)] pub fn append_with_fault(.., fail_after: Option<usize>)` for the crash test.
- Event kinds used by this slice: `evaluator.version.created`, `anchor.suite.verified`, `evaluator.challenger.completed`, `evaluation.completed`, `evaluator.promoted`, `evaluator.epoch.rotated`, `evaluation.marked_stale`. Fields always include `repo` (from `EvaluationLayout`, the repo root's final path component), and whichever of `slot`, `evaluator`, `epoch`, `trial`, `promotion`, `evaluation`, `suite`, `digest` apply. These names match handoff §18 and are additive kinds for the `autospec.events.v1` envelope when slice 4 forwards them via `emit_event`.

- [ ] **Step 1: Failing unit tests** (inline, on a temp layout):

```rust
#[test]
fn append_is_idempotent_by_key_and_chained() {
    let (layout, _guard) = temp_layout();
    let mut j = Journal::open(&layout).unwrap();
    assert!(j.append(1, "evaluator.version.created", "evaluator:architecture@1", fields(&[("slot", "architecture")])).unwrap());
    assert!(!j.append(2, "evaluator.version.created", "evaluator:architecture@1", fields(&[("slot", "architecture")])).unwrap());
    assert_eq!(j.append(3, "evaluator.version.created", "evaluator:architecture@1", fields(&[("slot", "documentation")])).unwrap_err().kind, EvaluationErrorKind::Integrity);
    assert_eq!(j.high_watermark(), 1);
    let reopened = Journal::open(&layout).unwrap();
    assert_eq!(reopened.events().len(), 1);
    assert!(reopened.contains("evaluator:architecture@1"));
}
#[test]
fn tampered_journal_fails_closed_on_open() {
    let (layout, _guard) = temp_layout();
    let mut j = Journal::open(&layout).unwrap();
    j.append(1, "k", "a", BTreeMap::new()).unwrap();
    j.append(2, "k", "b", BTreeMap::new()).unwrap();
    let text = std::fs::read_to_string(layout.journal_file()).unwrap().replace("\"key\":\"a\"", "\"key\":\"z\"");
    std::fs::write(layout.journal_file(), text).unwrap();
    assert_eq!(Journal::open(&layout).unwrap_err().kind, EvaluationErrorKind::Integrity);
}
#[test]
fn torn_append_is_rolled_back_and_checkpoint_stays_consistent() {
    let (layout, _guard) = temp_layout();
    let mut j = Journal::open(&layout).unwrap();
    j.append(1, "k", "a", BTreeMap::new()).unwrap();
    assert!(j.append_with_fault(2, "k", "b", BTreeMap::new(), Some(5)).is_err());
    let reopened = Journal::open(&layout).unwrap();
    assert_eq!(reopened.high_watermark(), 1);
    assert!(!reopened.contains("b"));
}
```

- [ ] **Step 2: Run to verify failure.** **Step 3: Implement** (serialize each event as one compact JSON line with `serde_json::to_vec` + `\n`; the digest covers the line bytes without the newline, matching `extend_journal_digest` in `managed_project.rs:200`). **Step 4: Run tests:** pass. **Step 5: Commit:** `git commit -am "feat(evaluation): hash-chained idempotent event journal"`.

---

### Task 11: Epoch transition transaction with crash recovery

**Files:**
- Create: `crates/autospec-core/src/evaluation/store/transition.rs`
- Create: `crates/autospec-core/tests/evaluation_transition.rs`
- Modify: `store/mod.rs` (`pub mod transition;`; `open()` calls `recover_pending_promotions()`)

**Interfaces:**
- Produces: `TransitionStep::{WritePromotionPending, WriteEpoch, MarkStale, AppendEvents, SwitchCurrent, CommitPromotion}` with `TransitionStep::ALL`; `Faults { fail_after: Option<TransitionStep> }` (`Default` = none); `impl EvaluationStore { pub fn promote(&mut self, trial: &ChallengerTrialId, approval: Approval, at: u64) -> Result<PromotionEvent>; pub fn pin(&mut self, evaluator: EvaluatorVersionRef, approval: Approval, at: u64) -> Result<PromotionEvent>; #[doc(hidden)] pub fn promote_with_faults(&mut self, trial, approval, at, faults: &Faults) -> Result<PromotionEvent>; pub fn recover_pending_promotions(&mut self) -> Result<Vec<PromotionId>>; }`.

`promote` = load trial → `plan_promotion(policy, current_epoch, trial, stale_candidates(records, slot, incumbent_version), approval, at)` → verify the challenger definition exists in the registry (`FailClosed` otherwise, handoff §22 "unknown evaluator version digest → no promotion") → `run_transition(promotion, faults)`.

`run_transition` performs the steps in order; each is idempotent so recovery can re-run all of them; after performing a step, if `faults.fail_after == Some(step)` return `Err(EvaluationError::fail_closed("injected fault after <step>"))`:

1. `WritePromotionPending` — `atomic_write` `promotions/<id>.json` with `state: pending` (overwriting a pending file with identical content is fine).
2. `WriteEpoch` — if `epochs/<new>.json` exists, read it and require `promotion == Some(id)` (else `Integrity`); otherwise `write_immutable_json` of `current.successor(...)`.
3. `MarkStale` — for each id in `invalidated_evaluations`: read record, `mark_stale(to, new_epoch, id, at)`, `atomic_write` only if changed.
4. `AppendEvents` — journal `evaluator.promoted` (key `promotion:<id>:promoted`), `evaluator.epoch.rotated` (key `promotion:<id>:rotated`), and one `evaluation.marked_stale` per record (key `evaluation:<ev>:stale:<promotion>`); dedup keys make re-runs no-ops.
5. `SwitchCurrent` — if `current.epoch_id < new_epoch`, `atomic_write` `current.json` (the single point where the transition becomes visible).
6. `CommitPromotion` — `atomic_write` the promotion with `state: committed`, `committed_at: Some(at)`.

`recover_pending_promotions` scans `promotions/*.json`, and for each `state: pending` re-runs `run_transition` with no faults; `open()` calls it so a crashed transition completes on the next command. Invariant: because `current.json` is one atomic rename, no observer ever sees two active epochs.

- [ ] **Step 1: Failing integration test** `crates/autospec-core/tests/evaluation_transition.rs`:

```rust
fn seeded_store(root: &Path) -> EvaluationStore {
    // init; register architecture@1 and architecture@2; register fixture suite; pin architecture@1 (creates epoch-000001);
    // write three records: ev-1 and ev-2 judged by architecture@1, ev-3 by documentation@1; write the qualified v1->v2 trial.
}
fn human() -> Approval { Approval { kind: ApprovalKind::Human, actor: "operator".into(), at: 500 } }

#[test]
fn full_promotion_rotates_epoch_and_marks_only_displaced_records() {
    let root = TempProjectRoot::new();
    let mut store = seeded_store(root.path());
    let trial_id = fixture_trial_id();
    let promotion = store.promote(&trial_id, human(), 600).unwrap();
    assert_eq!(promotion.state, PromotionState::Committed);
    assert_eq!(store.current_epoch().unwrap().epoch_id, EpochId(2));
    assert_eq!(store.current_epoch().unwrap().version_of(EvaluatorSlot::Architecture), Some(2));
    let records = store.records().unwrap();
    let stale = records.iter().filter(|r| matches!(r.active_ranking_status, ActiveRankingStatus::StaleForActiveRanking { .. })).count();
    assert_eq!(stale, 2);
    assert!(records.iter().all(|r| r.outcome == Verdict::Accept), "judgments are never rewritten");
    assert!(store.epoch(EpochId(1)).is_ok(), "history is retained");
    assert_eq!(store.promote(&trial_id, human(), 601).unwrap_err().kind, EvaluationErrorKind::FailClosed, "incumbent no longer matches");
}

#[test]
fn promotion_fails_closed_without_human_approval_for_architecture() {
    let root = TempProjectRoot::new();
    let mut store = seeded_store(root.path());
    let policy_approval = Approval { kind: ApprovalKind::Policy, actor: "policy".into(), at: 500 };
    assert_eq!(store.promote(&fixture_trial_id(), policy_approval, 600).unwrap_err().kind, EvaluationErrorKind::FailClosed);
    assert_eq!(store.current_epoch().unwrap().epoch_id, EpochId(1));
    assert!(store.promotions().unwrap().is_empty(), "nothing is persisted before planning succeeds");
}

#[test]
fn crash_after_every_step_recovers_to_exactly_one_active_epoch() {
    for step in TransitionStep::ALL {
        let root = TempProjectRoot::new();
        let mut store = seeded_store(root.path());
        let err = store.promote_with_faults(&fixture_trial_id(), human(), 600, &Faults { fail_after: Some(step) }).unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::FailClosed, "{step:?}");
        drop(store);
        let store = EvaluationStore::open(root.path()).unwrap(); // recovery runs here
        assert_eq!(store.current_epoch().unwrap().epoch_id, EpochId(2), "after {step:?}");
        let promotions = store.promotions().unwrap();
        assert_eq!(promotions.len(), 1, "after {step:?}");
        assert_eq!(promotions[0].state, PromotionState::Committed, "after {step:?}");
        let stale = store.records().unwrap().iter().filter(|r| !r.depends_on(EvaluatorSlot::Architecture, 1)).count();
        assert_eq!(stale, 3, "after {step:?}: two displaced + one unrelated");
        let promoted_events = store.journal_events().iter().filter(|e| e.kind == "evaluator.promoted").count();
        assert_eq!(promoted_events, 1, "after {step:?}: events appear exactly once");
        let mut store = store;
        assert!(store.recover_pending_promotions().unwrap().is_empty(), "second recovery is a no-op after {step:?}");
    }
}

#[test]
fn pin_seeds_an_empty_slot_and_refuses_an_occupied_one() {
    let root = TempProjectRoot::new();
    let mut store = seeded_store(root.path());
    let doc = EvaluatorVersionRef::parse("documentation@1").unwrap();
    store.register_evaluator(&fixture_definition_for(doc)).unwrap();
    let pin = store.pin(doc, Approval { kind: ApprovalKind::Policy, actor: "policy".into(), at: 1 }, 700).unwrap();
    assert_eq!(pin.from, None);
    assert_eq!(store.current_epoch().unwrap().slot_versions.len(), 2);
    assert!(store.pin(EvaluatorVersionRef::parse("architecture@2").unwrap(), human(), 701).is_err());
}
```

(`journal_events()` is a small read accessor on `EvaluationStore` returning `&[JournalEvent]`; add it in this task.)

- [ ] **Step 2: Run to verify failure.** **Step 3: Implement** `transition.rs` per the step list. **Step 4: Run** `cargo test -p autospec-core --test evaluation_transition` → pass, then `cargo test --workspace --no-fail-fast`. **Step 5: Commit:** `git commit -am "feat(evaluation): atomic epoch promotion with per-step crash recovery"`.

---
### Task 12: CLI group `autospec evaluator`

**Files:**
- Create: `crates/autospec-cli/src/commands/evaluator.rs`, `commands/evaluator/args.rs`, `commands/evaluator/registry_cmds.rs`, `commands/evaluator/epoch_cmds.rs`, `commands/evaluator/challenger_cmds.rs`, `commands/evaluator/promote_cmds.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs` — add `pub mod evaluator;`, the row `("evaluator", "Manage versioned evaluators, epochs, and promotions")` in `COMMANDS`, and the arm `"evaluator" => evaluator::run(rest),` in `run`.

**Interfaces:**
- Consumes: everything in `autospec_core::evaluation`.
- Produces the command surface below. Every subcommand accepts `--root <path>` (default `.`) and `--json`. Diagnostics use `CommandFailure::diagnostic` (exit 2). `EvaluationError` maps to `CommandFailure::diagnostic(format!("{error}"))`; the `fail-closed:` prefix from `Display` is what operators grep for.

```text
autospec evaluator init [--policy <file.json>]            writes policy.json (default policy when omitted), epoch-000000, current.json
autospec evaluator register --file <definition.json>      validates, writes v<N>.json, prints the definition digest
autospec evaluator list                                   one line per version: slot@version kind digest[..16] created_at
autospec evaluator show <slot@version>                    pretty JSON of the definition plus its digest
autospec evaluator pin <slot@version> --actor <name>      seeds an empty slot; creates the next epoch
autospec evaluator epoch current                          epoch id, started_at, policy digest, one line per slot
autospec evaluator epoch history                          every epoch, oldest first, with its promotion id
autospec evaluator challenger run --incumbent <slot@v> --challenger <slot@v> --suite <suite-id@v> \
        --incumbent-verdicts <file.json> --challenger-verdicts <file.json>
                                                          loads the suite with AccessRole::Qualification, pairs, qualifies, writes trials/<id>.json,
                                                          prints the report (verdict, BB values, margin, paired counts, subsets) and the trial id
autospec evaluator challenger inspect <trial-id>          pretty JSON of the trial
autospec evaluator promote <trial-id> [--approve --actor <name>]
                                                          Approval::Human when --approve --actor given, else Approval::Policy; prints the promotion
autospec evaluator record add --file <record.json>        writes a record (write-once)
autospec evaluator record list [--active | --stale]       one line per record: id evaluator epoch outcome status
```

- [ ] **Step 1: Write the failing CLI test** `crates/autospec-cli/tests/evaluator_commands.rs` covering the handoff §32 Step F demo end to end (binary invoked via `Command::new(env!("CARGO_BIN_EXE_autospec"))`; temp root via `tests/support/temp_directory.rs::unique`; fixtures copied from `crates/autospec-core/tests/fixtures/evaluation` — reference them by `concat!(env!("CARGO_MANIFEST_DIR"), "/../autospec-core/tests/fixtures/evaluation")`):

```rust
fn autospec(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autospec")).args(args).arg("--root").arg(root).output().expect("autospec runs")
}
fn ok(root: &Path, args: &[&str]) -> String {
    let out = autospec(root, args);
    assert!(out.status.success(), "{} failed:\n{}", args.join(" "), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
fn fails(root: &Path, args: &[&str]) -> String {
    let out = autospec(root, args);
    assert_eq!(out.status.code(), Some(2), "{} should fail closed", args.join(" "));
    String::from_utf8(out.stderr).unwrap()
}

#[test]
fn step_f_demo_registers_qualifies_promotes_and_retains_history() {
    let root = unique("autospec-evaluator-demo");
    stage_fixtures(&root); // copies anchors + verdicts + evaluator definition JSONs + record JSONs into <root>/fixtures
    ok(&root, &["evaluator", "init"]);
    ok(&root, &["evaluator", "register", "--file", "fixtures/evaluators/architecture-v1.json"]);
    ok(&root, &["evaluator", "register", "--file", "fixtures/evaluators/architecture-v2.json"]);
    let stderr = fails(&root, &["evaluator", "register", "--file", "fixtures/evaluators/architecture-v1.json"]);
    assert!(stderr.contains("immutable"), "{stderr}");
    ok(&root, &["anchor", "register", "--file", "fixtures/anchors/architecture-fixture/v1.json"]);
    ok(&root, &["evaluator", "pin", "architecture@1", "--actor", "operator"]);
    assert!(ok(&root, &["evaluator", "epoch", "current"]).contains("architecture@1"));
    for record in ["ev-1", "ev-2", "ev-3"] {
        ok(&root, &["evaluator", "record", "add", "--file", &format!("fixtures/records/{record}.json")]);
    }
    let report = ok(&root, &["evaluator", "challenger", "run", "--incumbent", "architecture@1", "--challenger", "architecture@2",
        "--suite", "architecture-fixture@1", "--incumbent-verdicts", "fixtures/verdicts/architecture-v1.json",
        "--challenger-verdicts", "fixtures/verdicts/architecture-v2.json"]);
    assert!(report.contains("verdict: qualified"), "{report}");
    assert!(report.contains("621494") && report.contains("790495"), "{report}");
    let trial_id = report.lines().find_map(|l| l.strip_prefix("trial: ")).unwrap().trim().to_string();
    let stderr = fails(&root, &["evaluator", "promote", &trial_id]);
    assert!(stderr.contains("fail-closed") && stderr.contains("human approval"), "{stderr}");
    let promoted = ok(&root, &["evaluator", "promote", &trial_id, "--approve", "--actor", "operator"]);
    assert!(promoted.contains("epoch-000002"), "{promoted}");
    let current = ok(&root, &["evaluator", "epoch", "current"]);
    assert!(current.contains("epoch-000002") && current.contains("architecture@2"), "{current}");
    let stale = ok(&root, &["evaluator", "record", "list", "--stale"]);
    assert!(stale.contains("ev-1") && stale.contains("ev-2") && !stale.contains("ev-3"), "{stale}");
    let active = ok(&root, &["evaluator", "record", "list", "--active"]);
    assert!(active.contains("ev-3"), "{active}");
    let history = ok(&root, &["evaluator", "epoch", "history"]);
    assert!(history.contains("epoch-000000") && history.contains("epoch-000001") && history.contains("epoch-000002"), "{history}");
}

#[test]
fn regressing_and_tied_challengers_never_promote() {
    let root = unique("autospec-evaluator-noprom");
    stage_fixtures(&root);
    ok(&root, &["evaluator", "init"]);
    for v in ["v1", "v2-regress", "v3-tie"] { ok(&root, &["evaluator", "register", "--file", &format!("fixtures/evaluators/architecture-{v}.json")]); }
    ok(&root, &["anchor", "register", "--file", "fixtures/anchors/architecture-fixture/v1.json"]);
    ok(&root, &["evaluator", "pin", "architecture@1", "--actor", "operator"]);
    let rejected = ok(&root, &["evaluator", "challenger", "run", "--incumbent", "architecture@1", "--challenger", "architecture@3",
        "--suite", "architecture-fixture@1", "--incumbent-verdicts", "fixtures/verdicts/architecture-v1.json",
        "--challenger-verdicts", "fixtures/verdicts/architecture-v2-regress.json"]);
    assert!(rejected.contains("verdict: rejected") && rejected.contains("critical"), "{rejected}");
    let trial = rejected.lines().find_map(|l| l.strip_prefix("trial: ")).unwrap().trim().to_string();
    assert!(fails(&root, &["evaluator", "promote", &trial, "--approve", "--actor", "operator"]).contains("fail-closed"));
    assert!(ok(&root, &["evaluator", "epoch", "current"]).contains("epoch-000001"));
}
```

(`architecture-v2-regress.json` and `architecture-v3-tie.json` verdict files declare `"evaluator": "architecture@3"` and `"architecture@4"` respectively so they can be registered as distinct versions; add matching definition fixtures `architecture-v2-regress.json` (version 3) and `architecture-v3-tie.json` (version 4).)

- [ ] **Step 2: Run to verify failure:** `cargo test -p autospec-cli --test evaluator_commands` → fails with "unknown command".

- [ ] **Step 3: Implement.** `evaluator.rs` mirrors `explore.rs` dispatch:

```rust
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic("autospec evaluator requires a subcommand")),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => { print_help(); Ok(()) }
        [command, rest @ ..] if command == "init" => registry_cmds::init(rest),
        [command, rest @ ..] if command == "register" => registry_cmds::register(rest),
        [command, rest @ ..] if command == "list" => registry_cmds::list(rest),
        [command, rest @ ..] if command == "show" => registry_cmds::show(rest),
        [command, rest @ ..] if command == "pin" => registry_cmds::pin(rest),
        [command, rest @ ..] if command == "epoch" => epoch_cmds::run(rest),
        [command, rest @ ..] if command == "challenger" => challenger_cmds::run(rest),
        [command, rest @ ..] if command == "promote" => promote_cmds::promote(rest),
        [command, rest @ ..] if command == "record" => promote_cmds::record(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!("unknown autospec evaluator command: {command}"))),
    }
}
```

`args.rs` provides `CommonArgs { root: PathBuf, json: bool }`, `fn split_common(args: &[String]) -> Result<(CommonArgs, Vec<String>), CommandFailure>` (extracts `--root <p>` and `--json`, leaves the rest positional/flag pairs), `fn option(rest: &[String], name: &str) -> Option<String>`, `fn now() -> u64` (`SystemTime::now().duration_since(UNIX_EPOCH)`), and `fn open_store(common: &CommonArgs) -> Result<EvaluationStore, CommandFailure>`.

`challenger_cmds::run_trial` prints, in this exact order so the test can parse it:

```text
trial: ct-<16 hex>
incumbent: architecture@1  successes=30 failures=10 best_belief=621494
challenger: architecture@2 successes=36 failures=4 best_belief=790495
margin_ppm: 169001  challenger_only_correct=6 incumbent_only_correct=0
subset critical: cases=5 incumbent_errors=0 challenger_errors=0 false_accepts=0 ceiling_breached=false regressed=false
verdict: qualified
```

with `verdict: rejected (critical: ceiling_breached: 2 false accepts / 0 false rejects; critical: regressed_vs_incumbent: 2 > 0 + 0)` and `verdict: inconclusive (margin_below_minimum: 0 < 10000)` for the other outcomes.

`promote_cmds::promote` maps a missing `--approve` to `Approval { kind: Policy, actor: "policy" }`; the core returns `FailClosed` with the message `slot architecture requires human approval (--approve --actor <name>)`.

- [ ] **Step 4: Run tests:** `cargo test -p autospec-cli --test evaluator_commands` → pass (Task 13 must be complete for the `anchor register` step; implement 12 and 13 together if executing serially). **Step 5: Commit:** `git commit -am "feat(cli): autospec evaluator command group"`.

---

### Task 13: CLI group `autospec anchor`

**Files:**
- Create: `crates/autospec-cli/src/commands/anchor.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs` — `pub mod anchor;`, row `("anchor", "Register and verify protected evaluator anchor suites")`, arm `"anchor" => anchor::run(rest),`

**Interfaces:**

```text
autospec anchor register --file <suite.json>           validate + verify_artifacts against --root + write-once + journal
autospec anchor list                                   one line per suite version: id@version slot cases digest[..16]
autospec anchor show <suite-id@version> --role <operator|qualification|mutation>   pretty JSON of the role's view (default: mutation — the safe default)
autospec anchor verify <suite-id@version>              recompute artifact digests; exit 2 naming mismatched cases
```

- [ ] **Step 1: Failing CLI test** (append to `crates/autospec-cli/tests/evaluator_commands.rs`):

```rust
#[test]
fn anchor_show_defaults_to_the_redacted_view_and_verify_names_tampered_cases() {
    let root = unique("autospec-anchor");
    stage_fixtures(&root);
    ok(&root, &["evaluator", "init"]);
    ok(&root, &["anchor", "register", "--file", "fixtures/anchors/architecture-fixture/v1.json"]);
    let redacted = ok(&root, &["anchor", "show", "architecture-fixture@1"]);
    assert!(redacted.contains("\"case_id\": \"case-40\""), "holdout cases stay visible, only their labels are redacted:\n{redacted}");
    let full = ok(&root, &["anchor", "show", "architecture-fixture@1", "--role", "qualification"]);
    assert_eq!(full.matches("\"expected_label\": \"").count(), 40, "{full}");
    assert_eq!(redacted.matches("\"expected_label\": \"").count(), 30, "holdout cases 31-40 must be unlabeled:\n{redacted}");
    ok(&root, &["anchor", "verify", "architecture-fixture@1"]);
    std::fs::write(root.join("fixtures/anchors/architecture-fixture/cases/case-07/patch.diff"), b"poisoned").unwrap();
    let stderr = fails(&root, &["anchor", "verify", "architecture-fixture@1"]);
    assert!(stderr.contains("case-07"), "{stderr}");
}
```

- [ ] **Step 2: Run to verify failure.** **Step 3: Implement** with the same dispatch shape as Task 12; `show` parses `--role` through `AccessRole::parse` (add `parse`/`as_str` to `AccessRole` in `anchor.rs` if not present: `operator`, `qualification`, `mutation`). **Step 4: Run tests:** pass; run `cargo test --workspace --no-fail-fast`. **Step 5: Commit:** `git commit -am "feat(cli): autospec anchor command group with redacted default view"`.

---

### Task 14: Documentation, gates, and validation

**Files:**
- Modify: `docs/cli-reference.md` (new `evaluator` and `anchor` sections mirroring the tables in Tasks 12–13), `docs/CONFIG_REFERENCE.md` (new `.autospec/evaluation/policy.json` section with the default policy JSON and each field's meaning), `docs/architecture.md` (one subsection "Evaluator epochs" under the Rust core description: what is protected, what is immutable, where state lives, pointer to ADR 0002), `README.md` (one bullet under the CLI overview if a CLI list exists there)
- Verify: `.autospec/architecture-fitness.yml` unchanged; `crates/autospec-core/src/validation/structural.rs` needs no change (no new required files); `docs/index.md` links the new spec if it indexes specs.

- [ ] **Step 1: Write the docs.** In `docs/CONFIG_REFERENCE.md` include:

```json
{
  "schema": 1,
  "epsilon": 50000,
  "minimum_margin": 10000,
  "minimum_cases": 40,
  "require_human_approval_slots": ["architecture", "security_reasoning"]
}
```

with the sentence: "`epsilon` and `minimum_margin` are parts per million. A challenger is promoted only when its ε-best-belief lower bound exceeds the incumbent's by at least `minimum_margin` on at least `minimum_cases` labeled anchors with no protected-subset regression; slots listed under `require_human_approval_slots` additionally need `autospec evaluator promote --approve --actor <name>`."

- [ ] **Step 2: Run every gate:**

```bash
cargo fmt --all -- --check   # format only the new files if the repo has baseline debt (issue #3536); do not reformat unrelated files
cargo clippy --workspace --all-targets
cargo test --workspace --no-fail-fast
bash scripts/architecture-fitness.sh run --registry .autospec/architecture-fitness.yml   # financial_no_f64 observed must still be 81
cargo run -p autospec-cli -- validate --fast
awk 'END { if (NR > 600) exit 1 }' crates/autospec-core/src/evaluation/*.rs crates/autospec-core/src/evaluation/store/*.rs crates/autospec-cli/src/commands/evaluator/*.rs   # per-file, loop in shell
```

- [ ] **Step 3: Commit:** `git commit -am "docs: evaluator epochs CLI, policy reference, and architecture note"`.

- [ ] **Step 4: Open the PR** from the worktree (`gh pr create --base main --head feat/evaluator-coevolution-slice-1 --title "feat: versioned evaluators, frozen epochs, and controlled promotion (slice 1)" --body-file <file>` whose body contains `Source spec: docs/specs/2026-09-05-evaluator-coevolution-design.md`, the validation commands and outcomes, and the note that `financial_no_f64` remained at 81). Per CONTRIBUTING.md, call out the most likely hidden failure: fixture path relocation in `stage_fixtures` on Windows CI.

---

## Part D — Companion repositories (manual PRs; the V65 sync bridge is proposal-only)

### Task 15: autospec-constitution — evaluator governance clauses (0.6.1 → 0.7.0)

**Files (in a clone of `berlinguyinca/autospec-constitution`):**
- Modify: `docs/19-review-and-critique-doctrine.md` (add under `## Rules` a subsection `### Learned evaluators and epochs`, and matching entries under `## Quality Gates` and `## Anti-Patterns`; keep the canonical nine-section order the validator enforces)
- Modify: `CONSTITUTION.md` (`Version: 0.7.0`), `CHANGELOG.md` (new `## 0.7.0 - 2026-09-05` entry classified minor), `schemas/constitution.schema.json` (`version` default `0.7.0`)

- [ ] **Step 1: Add exactly these three clause families** (doctrine 19 already states "the gate wins" and "never the sole approver"; do not restate them):

```markdown
### Learned evaluators and epochs

- **Evaluator versions are immutable within an active epoch.** A learned reviewer (a prompt, rubric, model constraint, tool policy, and knowledge base taken together) is identified by a version and a content digest. Changing any of those creates a new version; nothing edits an active version in place.
- **Replacement is evidence-gated against protected anchors.** A challenger evaluator replaces an incumbent only at an explicit epoch boundary, only after outperforming the incumbent on a labeled anchor suite whose protected-holdout labels the challenger's authors and any mutation process could not read, and only when no protected subset regresses. A tie or an incomplete comparison never promotes.
- **Evaluator history is append-only and spend is ceilinged.** Replacing an evaluator marks the judgments it produced as stale for active ranking; it never deletes them, and both the historical and the current judgment stay inspectable. Evaluator qualification and any automated evaluator search run under an explicit budget ceiling and stop, resumably, when it is exhausted.
```

Quality gate: "A promotion record names the challenger trial, the anchor suite digest, the policy digest, the approver, and every judgment it invalidated." Anti-pattern: "Re-running a challenger until a favourable sample appears, or editing anchor labels after seeing a result."

- [ ] **Step 2: Run** `python3 scripts/validate-constitution.py` (checks section order and the three version markers). **Step 3: PR** titled `feat: evaluator governance clauses (0.7.0)`; body links `berlinguyinca/autospec` ADR 0002. This PR lands **before** Task 16, because baselines CI resolves `doctrine_refs` against a live checkout of the constitution.

### Task 16: autospec-baselines — evaluator qualification rules and seed anchor pack

**Files (in a clone of `berlinguyinca/autospec-baselines`):**
- Create: `docs/rules/evaluator-qualification.rules.yaml`
- Create: `docs/rules/anchor-fixtures/architecture-v1/manifest.json`, `docs/rules/anchor-fixtures/architecture-v1/cases/<id>/patch.diff` (the 40-case fixture from Task 7, copied verbatim; `artifact_ref` values are relative to the `architecture-v1/` directory), `docs/rules/anchor-fixtures/README.md`
- Modify: `scripts/validate-packs.py` (walk `docs/rules/anchor-fixtures/*/manifest.json`; require unique `case_id`s, every `artifact_ref` to exist, and each file's SHA-256 to equal `content_digest`)

- [ ] **Step 1: Write the rules registry** in the existing format:

```yaml
version: 1
domain: evaluator-qualification
constitution_min_version: 0.7.0
doctrine_refs: [19]
rubric: [independence, protected anchors, conservative lower bound, append-only history]
rules:
  - id: evq.version-immutable
    doctrine: 19
    category: review
    severity: blocker
    check: auto
    tool: autospec evaluator register
    pass: "registering an existing slot@version exits non-zero"
  - id: evq.holdout-redacted
    doctrine: 19
    category: review
    severity: blocker
    check: auto
    tool: autospec anchor show --role mutation
    pass: "protected_holdout cases carry no expected_label"
  - id: evq.promotion-lower-bound
    doctrine: 19
    category: review
    severity: blocker
    check: auto
    tool: autospec evaluator challenger run
    pass: "verdict is qualified only when challenger best-belief exceeds incumbent by the policy margin with no subset regression"
  - id: evq.history-retained
    doctrine: 19
    category: review
    severity: major
    check: auto
    tool: autospec evaluator record list --stale
    pass: "displaced judgments are listed as stale, never absent"
```

- [ ] **Step 2: Extend `validate-packs.py`**, run it, and confirm `gen-pack-doc.py --all --check` still passes (no `.pack.json` is added, so no generated twin is needed). **Step 3: PR** titled `feat: evaluator qualification rules and architecture anchor fixture v1`. The README states that the fixture is synthetic, that repo-derived anchors stay in the target repository's `.autospec/evaluation/anchors/`, and that AutoSpec pins the version it activates.

---

## Part E — Follow-up slices (not tasks; each gets its own plan when its gate clears)

| Slice | Scope | Gate (must be true before planning) |
|---|---|---|
| 2 — live judges | `VerdictSource` implementation that dispatches a review-only role through the executor abstraction (#3172 landed as PR #3196) with `RuntimeProvenance` filled from `review_provider.rs` (`providers_are_diverse`), unknown provider ⇒ `Independence::None`; `routing_policy_digest` derived from `ModelProfileRegistry` version | #3173 or an equivalent callable review-only dispatch exists; #3531's attestation record has a Rust type to reuse |
| 2b — orchestrator trials | submit qualification trials as `ExecutionManifest`s with `labels`, `timeout_seconds`, `required_artifacts`; map `NO-OUTPUT`/`STALLED` to `Verdict::Unavailable` | orchestrator issues #2, #4, #6–#8, #11–#13, #15, #20, #23, #25 merged; `shared-contracts.md` amended for the five additions listed in Part A.4 |
| 3 — bounded AVO variation | candidate manifests + lineage for one mutable target (a review skill prompt), variation loop as a capability-gated conductor tier scored by the shared WSJF ranker, stagnation as a quality-plateau signal, trio + goldens regenerated in the same commit | a `diff-guard` lane for the declared mutable surface with its own provenance; slice 2 live judges qualified in shadow mode on the fixture suite |
| 4 — telemetry + GUI | forward journal kinds through `emit_event`; `autospec-db` `migrations/004_evaluation_views.sql` as plain views over `events_raw` (no materialized views; explicit `grant execute` for any function); GUI page + `AUTOSPEC_TELEMETRY_SCHEMA=autospec` + view names added to `telemetry.ts` constants | slice 1 journal kinds stable for one release |
| 5 — incident mining | false-accept/false-reject capture from human review outcomes into `quarantine` cases, adjudication flow, replay | slice 2 producing real judgments |

### Task 17: Decomposition into issues (run after the PR from Task 14 merges)

- [ ] **Step 1:** `git fetch origin && git cat-file -e origin/main:docs/specs/2026-09-05-evaluator-coevolution-design.md` (the `/autospec-split` gate).
- [ ] **Step 2:** `/autospec-split docs/specs/2026-09-05-evaluator-coevolution-design.md`. Expected children, one per Task 2–14 above, each ≤ 400 words, ≤ 3 logical units, `Depends on issue #N` in task order, labelled by Phase 3.5 (`ctx:64k` for Tasks 7, 9, 11, 12; `ctx:32k` for the rest; `reasoning:deep` for Tasks 3, 7, 11; `reasoning:medium` otherwise; `lang:rust`). Record the parent under tracker #3381 with `autospec parent record`.
- [ ] **Step 3:** File the two companion-repo PRs (Tasks 15–16) by hand; `/autospec-define` cannot file cross-repo issues.

---

## Self-review

**Spec coverage (handoff §33 first-slice deliverables):** typed evaluator/epoch/anchor models → Tasks 4, 5, 8; local persistence → Tasks 9–11; schemas → serde contracts with `deny_unknown_fields` plus `schema` fields, asserted by tests (no `schemas/*.json` file: nothing validates them in Rust today, see Part A.5); CLI read/write for fixtures → Tasks 12–13; qualification statistic → Task 3; promotion transaction → Task 11; stale marking/replay metadata → Task 8 (`ActiveRankingStatus::Replayed` exists; replay *scheduling* is slice 2); deterministic tests → every task; docs → Task 14; constitution clauses → Task 15; baselines fixture pack → Task 16. Handoff §9.5 step 6 "schedule required replay" is deferred with the live judge (slice 2) and stated so. Handoff §16.9 E2E scenarios depend on the orchestrator and are slice 2b.

**Placeholder scan:** no deferred-work markers of any kind; every code step shows code; Task 9's `copy_fixture_suite_into` and Task 11's `seeded_store` are test helpers whose behaviour is spelled out in comments and whose inputs are the Task 7 fixtures.

**Type consistency:** `Ppm(pub u32)` everywhere; `EvaluatorVersionRef { slot, version: u32 }` used by Tasks 4, 7, 8, 11, 12; `AccessRole::{Operator, Qualification, Mutation}` in Tasks 5, 9, 13; `ChallengerVerdict` variants match between Tasks 7 and 8; `Approval`/`ApprovalKind` match between Tasks 8, 11, 12; `TransitionStep::ALL` and `Faults { fail_after }` match between Tasks 11 and its test; journal kinds in Task 10 match those emitted in Tasks 9 and 11.
