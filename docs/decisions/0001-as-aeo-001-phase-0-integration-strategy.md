# ADR 0001 — AS-AEO-001 Phase 0 integration strategy

- **Date:** 2026-08-16
- **Status:** Accepted
- **Satisfies:** `docs/specs/2026-08-16-autonomous-engineering-organization-design.md` §73 Phase 0
  ("map existing modules … identify reusable components; create ADR for integration strategy"),
  whose exit condition is *"approved migration map; no duplicate subsystem implementation
  started prematurely."*
- **Supersedes decomposition assumptions in:** `docs/specs/2026-08-16-multi-model-engineering-team-design.md`

## Context

Three specifications landed on `main` on 2026-08-16:

| Spec | State |
|---|---|
| `2026-08-16-multi-model-engineering-team-design.md` | Decomposed into issues #3162–#3176 (bash) |
| `2026-08-16-benchmark-per-evaluation-telemetry-design.md` | Not decomposed |
| `2026-08-16-vision-image-generation-qualification-design.md` | Not decomposed |
| `2026-08-16-autonomous-engineering-organization-design.md` (AS-AEO-001) | Not decomposed — gated by this ADR |

AS-AEO-001 subsumes the concerns of all three and mandates Rust (§64 persistence,
§65 core types, §66 module architecture). The already-filed issues implement the
same subsystems in bash. Without this ADR, the two programs would build one
control plane twice.

## Current-state findings

Established by audit of the tree at `56aa090b`. Findings that changed the decision
are marked **load-bearing**.

### The bash routing layer is not on the execution path — load-bearing

`scripts/route-decide.sh` (352 lines) has **no executable caller**. References are
`README.md`, `docs/USER_MANUAL.md`, `docs/CONFIG_REFERENCE.md`, and two prose
comments in `scripts/calibrate-profile.sh` and `scripts/verify-voter-vendor.sh`.
`scripts/dispatch-implementer.sh` consults no routing helper at all.

The routing/provider bash surface is ~2,264 lines across 10 files and is
**advisory scaffolding beside the pipeline, not inside it**. Live dispatch runs
through Rust `executor_bridge` plus `dispatch-implementer.sh`.

Consequence: the usual argument for preserving the bash layer — *it works, don't
break it* — does not apply. Wrapping a subsystem nothing calls buys backward
compatibility with nothing, and §72.3 shadow operation has no "current
assignments" stream to compare against.

Tracked as a defect in its own right: **#3179**.

**Resolution (#3179):** documented as advisory tooling rather than wired into
`scripts/dispatch-implementer.sh`. `dispatch-implementer.sh` has no
model-selection logic or `--labels` input today, so a shadow invocation there
would mean inventing new plumbing on the critical dispatch path to log a
decision nothing currently compares against — not restoring an existing call
site. This is also the option consistent with this ADR's own finding above
that the bash routing layer is "advisory scaffolding beside the pipeline, not
inside it." `README.md`, `docs/USER_MANUAL.md`, and `docs/CONFIG_REFERENCE.md`
now say so explicitly; `scripts/route-decide.sh`'s decision logic is unchanged.

### There is no database — load-bearing for cost

No `sqlx` / `rusqlite` / `diesel` / `postgres` dependency in any `Cargo.toml`.
No `migrations/`. State is JSON files (`crates/autospec-core/src/state/storage.rs`)
plus append-only JSONL under `~/.autospec/`.

`autospec-db` is **not in this repo** — it is an optional external binary, reached
through a 53-line fire-and-forget shim (`skills/autospec-shared/scripts/emit-event.sh`),
write-only, and contractually a no-op when `AUTOSPEC_DB_DSN` is unset.

Consequence: §64 introduces persistence **from scratch**, and §67's `async fn`
provider trait additionally introduces an async runtime. Both enter a workspace
with six pinned dependencies and no async. This is a Phase 1 cost to budget, not
a schema exercise.

### Orchestration is already Rust

`core::{coordination, autonomous, execution, claim}` ≈ 16,400 LOC; `cli::commands::autonomous`
including `executor_bridge` ≈ 53,600 LOC. AS-AEO-001's Rust mandate **agrees** with
where orchestration lives and disagrees only with where the disconnected routing
scaffolding lives.

### Several AS-AEO-001 epics describe shipped code

Epic 11 ("proposal state", "policy-controlled promotion") already exists as
`schemas/autospec-explore-proposal.schema.json`, `core::explore` (1,522 LOC),
tiers 2/3/4, and `tier15`/`issue_promotion`. Epic 12 overlaps
`autonomous-usage-governor.sh` and `autospec-stop`. Epic 9 overlaps the shipped
playwright/accessibility/UI-audit surface. Epic 7 overlaps `core::evidence` (485 LOC)
and three evidence schemas.

These epics must be rescoped to *formalize what exists* before decomposition.

### GitHub Projects V2 is genuinely greenfield

No `gh project` executable code exists in bash or Rust — only prose in
`skills/autospec-define/SKILL.md`. Nothing to be compatible with, and therefore
nothing lost by deferring it to whichever program builds it once.

## Decisions

### D1 — Rescope, do not supersede and do not layer wholesale

Keep the work that extends live, wired code; park the net-new bash subsystems that
AS-AEO-001 rebuilds in Rust; begin AS-AEO-001 at Phases 0–2, which have no overlap.

Rejected alternatives:

- **Supersede everything.** Discards #3167/#3172/#3173/#3174, which are precisely the
  inventory, adapter, and shadow-telemetry inputs §72.1–§72.3 require, and parks the
  repo's only concrete plan for local-model execution behind a ten-phase program.
- **Layer, keeping all 13.** "The bash layer is the §72.2 compatibility surface" holds
  only for #3172/#3173 and partly #3167/#3174. For the rest there is no existing
  behavior to be compatible with, so it pays for two implementations to get the
  compatibility value of one.

### D2 — Role vocabulary is 14 snake_case

`orchestrator planner architect test_planner implementer code_reviewer test_reviewer
qa_verifier documentation_writer documentation_reviewer ui_ux_reviewer
security_reviewer researcher advisor`.

§65's 21-variant CamelCase enum does not apply. §65 explicitly permits adapting
naming to the codebase, and the 14 names are already binding across every filed
issue body. The eight §65 roles without an autospec analogue — `RiskAssessor`,
`RoutingAdvisor`, `BenchmarkAnalyst`, `DependencyReviewer`, `AccessibilityReviewer`,
`IntegrationReviewer`, `ReleasePreparer`, `ReleaseReviewer` — are recorded here as a
**documented future extension**, not as a competing enum.

### D3 — The JSONL ledger is the system of record; any database is a projection

§64's `routing_decisions` / `execution_attempts` / `metrics` tables, if built, are a
derived read model over the append-only JSONL ledger. The "ONE ledger — extend,
never fork" rule binding on #3163–#3175 stands.

This keeps local-only operation working with no database present, which the
external, optional, no-op-by-default nature of `autospec-db` already requires.

### D4 — The disconnected router is a defect, not a design

Filed as #3179. Independent of D1: whichever language wins, documentation must not
describe an entry point that nothing invokes.

### D5 — Accept an async runtime and a database driver in Phase 1

*Added 2026-08-16, resolving Open item 2 of the original ADR.*

§64 (persistence) and §67 (`async fn` provider trait) are taken as written. Phase 1
therefore admits tokio, a database driver, and a migration tool into a workspace
whose current dependency set is six pinned crates (`serde`, `serde_json`, `sha2`,
`yaml-edit`, `getrandom`, `nix`) with no async runtime.

This is the larger of the two options and it is chosen deliberately. Bounding
conditions:

- **D3 still governs the data.** The database is a projection; the append-only JSONL
  ledger remains the system of record. Accepting a driver does not promote the
  database to authoritative.
- **Local-only operation must keep working with no database present.** This follows
  from D3 and from the existing contract that `autospec-db` is optional and a no-op
  without a DSN. A missing database degrades functionality, never correctness.
- **Dependency direction per §66.1.** The driver and runtime belong beneath the
  domain layer. Core policy, risk, and role types must not gain an async signature
  merely because a provider adapter is async.
- **The cost is real.** Budget Phase 1 for dependency-graph review, build-time
  increase, and the first async boundary in a codebase that has none — not for a
  schema exercise.

### D6 — The RealWork spec is authoritative for the benchmark subsystem

*Added 2026-08-16, resolving Open item 1 of the original ADR.*

`docs/specs/2026-08-16-repository-derived-real-work-benchmark-design.md` owns the
benchmark subsystem: corpus, historical-replay methodology, difficulty and scoring
model, qualification rules, the `autospec bench` CLI (§49), and
`crates/autospec-bench/` (§51).

The other three documents become layers under it rather than competitors:

| Document | Role after D6 |
|---|---|
| `2026-08-16-benchmark-per-evaluation-telemetry-design.md` | Metric layer — deepens RealWork §31 |
| `2026-08-16-vision-image-generation-qualification-design.md` | A task family alongside the RealWork families |
| AS-AEO-001 Epic 4 | Integration point only — ingests RealWork results; must not define a second benchmark system |

This holds D3: RealWork §53 appends to the existing ledger rather than creating a
benchmark store, so the JSONL ledger remains the system of record.

**Two hard gates carried into decomposition.** Phases 1–2 (corpus framework, public
seed corpus) are decomposable now. Phase 3 mines LC-BinBase Scheduler, the WCMC
applications, and private Go modules — real production repositories containing
credentials, customer data, and protected datasets — so RealWork §36 secret
detection and §37 access levels must exist, be tested against known-positive
fixtures, and be independently reviewed **before** any private repository is mined.
Phase 7 (router integration) waits on AS-AEO-001 Epic 5, since the deterministic
router was parked by D1.

### D7 — The resource ledger shares AS-AEO-001's persistence layer

*Added 2026-08-16, on landing `2026-08-16-resource-lifecycle-cleanup-design.md`.*

That spec's §12 defines a SQLite `resources` table with leases, heartbeats and
transactional updates. Two clarifications, so it is neither blocked by nor allowed
to duplicate existing decisions:

- **D3 does not apply to it.** D3 governs routing and dispatch telemetry — an
  append-only stream of immutable events. Resource records are mutable operational
  lifecycle state and are not log-shaped. Forcing them into the JSONL ledger would
  be wrong.
- **It is not a second database.** D5 already admitted a driver and migration tooling
  in AS-AEO-001 Phase 1. The `resources` table belongs in that persistence layer as
  additional tables and migrations. If the resource subsystem lands first, its storage
  must be written so those tables migrate into the shared database without a data
  migration.

Ownership split against AS-AEO-001: resource identity, leases, reconciliation,
janitor and cleanup verification belong to the resource spec; run/work-item
lifecycle, policy, risk, approvals, emergency stop and budgets remain AS-AEO-001
(§51, §69, Epic 12).

**Safety sequencing is part of this decision.** The host measured 6,475 local
branches, 5,926 of them already merged, 25 worktrees, and 13 Docker containers of
which none carry an `autospec` label. Phases 3-5 (cleanup, crash recovery, janitor)
must not be decomposed until the §42 git-safety and §45 property/invariant tests
exist; Invariants 1, 2, 3 and 5 are the acceptance bar. Phase 1 is observation and
dry-run only and is safe to start.

### D8 — The Change Graph consumes cleanup and routing; it does not define them

*Added 2026-08-16, on landing `2026-08-16-change-graph-pr-orchestration-design.md`.*

That spec's novel contribution is real and unbuilt — verified: no ChangeSet, Change
Graph or stacked-PR surface exists in `crates/`. Its core is the Change Graph and
ChangeSet abstractions, dependency edge strength, stack-selection policy, size
budgets, the PR contract, conflict-aware scheduling, three CI levels, merge
strategy/queue/bottom-up merge, and replanning.

Its §25-§31, however, restate the resource lifecycle subsystem — both documents
define `autospec cleanup --dry-run` and an `autospec.*` Docker label set. **The
resource lifecycle spec wins** (D7); it carries the lease, janitor, reconciliation
and eight safety invariants. A ChangeSet is an *owner identity in the resource
ledger*, not a second cleanup engine.

Likewise §17-§20 delegate to AS-AEO-001 Epics 5-6 (roles are D2's 14 snake_case set),
§21-§23 to Epic 8, §37-§39 to Epics 7/9 and the vision spec, and §48-§49 to the
existing fleet layer.

Two incumbents must be extended rather than recreated: `core::graph::order.rs`
(117 lines, topological ordering with cycle detection) for §14, and
`core::coordination::ready_queue.rs` for §15.1.

**Gate:** the novel core consumes the executor abstraction (#3172/#3173), the
resource ledger (resource-lifecycle Phase 1) and the router (Epic 5). Decompose it
only after the executor chain and resource-lifecycle Phase 1 have merged. The
delegated sections are never decomposed from that document.

### D9 — The control-plane spec contributes three workstreams; five delegate

*Added 2026-08-16, on landing `2026-08-16-next-generation-control-plane-design.md`.*

Of its eight workstreams, three are genuinely new and are the reason to implement it:
**A Context Compiler** (what to put in an agent's prompt — note the name collision with
the multi-model spec's §11 GPU KV-cache budgeting, an unrelated concern), **B Complexity
Governor** (the trigger for replanning, where change-graph §47 is the mechanism), and
**H Engineering Policy Compiler** (deriving policy, where AS-AEO-001 only enforces it).

Five delegate: **C** outcome learning to multi-model §24-§29 and D6, appending to the one
ledger per D3; **D** traceability to AS-AEO-001 §47/Epic 7 over the shipped
`core::evidence`; **E** tournament mode to multi-model §4 and D2 for independence rules;
**F** onboarding-calibration to the RealWork spec per D6; **G** multi-repo to the existing
fleet layer and change-graph §48-§49 per D8.

Its §14 eighteen record types persist in the ONE shared database (D5, D7) using the
resource ledger's shared-migration protocol. Its §15 provenance contract unifies with the
routing ledger's explainability output rather than becoming a parallel decision log.

**Gate:** only Workstream A has no unbuilt dependency. Nothing here is decomposable
before the executor chain (#3172/#3173) and resource-lifecycle Phase 1 merge.

**Standing note on planning cadence.** This was the eighth design landed on 2026-08-16
against one merged implementation PR. Every spec after the second has required a
precedence decision (D6, D7, D8, D9), each of the form "X restates Y; Y wins." The
delegation pattern is working, but the design surface is outrunning the implementation
surface, and this spec's own §1 argues the same: after these capabilities land, effort
should shift to implementation quality, dogfooding, benchmarks and reliability rather
than continued feature invention.

### D10 — Postgres compatibility is required; sqlx over rusqlite, database is global

*Added 2026-08-16. Supersedes the driver and location choices in issue #3188, and
tightens D5 and D7.*

**Postgres compatibility is a hard requirement**, not an eventual option. AS-AEO-001
§64 already said "SQLite with a migration path to PostgreSQL"; this makes it binding.

**Driver: `sqlx`, with the SQLite and Postgres backends.** `rusqlite` is rejected as
the shared persistence binding: it is a SQLite-only C-FFI API with no Postgres path, so
reaching Postgres later would mean rewriting every call site rather than swapping a
driver. The window to change this is now — once a ledger type embeds a concrete
`rusqlite::Connection`, the choice stops being cheaply reversible.

Rejected alternatives:
- **diesel** — synchronous and supports both backends, but backend-generic Diesel code
  is awkward (differing types, differing upsert syntax) on top of schema codegen.
- **rusqlite behind a storage trait** — cheapest today, but the Postgres path stays
  theoretical until a second implementation exists, and the SQL is written twice.
  A requirement is not satisfied by an abstraction that permits it.

**This accepts async into `autospec-core`.** sqlx is async-only, so tokio enters the
dependency graph. That is already sanctioned: D5 admitted a runtime and a driver, and
AS-AEO-001 §67 mandates `async fn` provider traits regardless. D5's bounding condition
still holds — per §66.1, async belongs at the storage boundary; core policy, risk and
role types must not gain async signatures because storage is async.

The blanket "no `tokio`, no `sqlx`" prohibition drafted into #3188 is **withdrawn**. It
was a project-wide constraint asserted by a single child issue, above that issue's
authority and contrary to D5.

**Location: one global database under `~/.autospec`.** A repo-local file cannot back a
shared Postgres server, cannot hold AS-AEO-001 §64's cross-repo `organizations` /
`projects` / `repositories` tables, and is already inconsistent with Phase 1 — sibling
issue #3191 reads global `~/.autospec/process-heartbeats/`, so Phase 1 spans both roots
either way. Repo-local `.autospec/` remains correct for the existing JSON state layer
(`state/storage.rs`, `evidence/`, `execution/`); it is the *database* that is global.

**D7 still governs:** exactly ONE database, shared with AS-AEO-001 Epic 2. The
shared-migration protocol D7 requires is now more important, not less — sqlx's migration
tooling must be configured so two subsystems can add migrations without colliding.

## Issue disposition

| Issue | Action | Rationale |
|---|---|---|
| #3167 discovery evidence + calibration | **keep** | Extends `discover-model-supply.sh` / `calibrate-profile.sh`, which exist and run; produces §72.1 inventory and Epic 3 qualification inputs |
| #3172 provider-neutral executor | **keep** | Literally §72.2 — wraps existing provider/execution paths behind an interface before behavior is replaced |
| #3173 local Qwen execution | **keep** | Only concrete plan for running local models; extends `local-dispatch.sh` in place |
| #3174 per-dispatch ledger fields | **keep** | Additive optional fields on an existing JSONL; the stream §72.3 shadow operation needs |
| #3175 smoothed statistics | **keep** | Substrate validated by D3; hierarchical backoff is unowned by AS-AEO-001 |
| #3176 Phase 5.5 audit | **rescoped** | Now audits only the surviving set; seams shifted to executor contract, ledger back-compat, evidence levels, fail-closed dispatch |
| #3163 roles + independence | **rewrite** | Behavior mandatory (§7.6, §19, AC-007); belongs in `core::safety` beside `review_policy.rs`, not a new bash script |
| #3164, #3165 GitHub Project + sync | **parked** | Epic 8, 1:1; Projects V2 is greenfield in both plans |
| #3166 capability schemas | **parked** | Epic 3 registry; typed Rust supersedes a hand-written JSON Schema envelope |
| #3168 context reservations | **parked** | Lowest-confidence call — §64 has no table for a host-local GPU lease. Revive under Epic 5 if local multi-worker scheduling is needed before Phase 4 |
| #3169 quota + health probes | **parked** | Epic 5 "provider-health interface", named identically |
| #3170, #3171 router + explainability | **parked** | Epic 5 "candidate filtering" / "route explanation"; §65 `RoutingRequest` is field-for-field #3170's CLI surface |

Carried forward into the Epic 6 re-file of #3163, because neither is stated in
§78's bullets and both are easy to lose: §4.3 (escalation removes the escalated
model from reviewer candidates) and §4.4 (independence is judged on underlying
model identity, not profile alias).

## Consequences

- AS-AEO-001 may begin at Phases 0–2. Epics 1, 3, 5, 6, 8 are unblocked for
  decomposition only after the epics that describe shipped code (7, 9, 11, 12) are
  rescoped to formalize rather than rebuild.
- The two unsplit 2026-08-16 amendment specs remain unsplit. They and AS-AEO-001
  Epic 4 are three documents describing one `autospec bench` subsystem that does not
  exist (`commands/benchmark.rs` is a 3-line stub). Splitting them before naming one
  authoritative would recreate this conflict in the benchmark layer.
- Parked issues keep their bodies as source material; only the substrate changes.

## Open items

1. ~~Which document is authoritative for the benchmark subsystem.~~
   **Resolved by D6 — the RealWork spec.**
2. ~~Whether tokio plus a database driver and migration tool are accepted into
   `autospec-core`'s dependency graph in Phase 1.~~ **Resolved by D5 — accepted.**
3. Whether `executor_bridge/waterfall_policy.rs` and `review_evidence.rs` encode
   policy rather than delegating to `core` — a suspected §66.1 violation
   ("provider adapters shall not contain policy logic"), unverified.
4. Evaluator co-evolution (handoff 2026-09-05) is reconciled in ADR 0002; its AS-AEO-001 Epic 3/4/6 touch points consume D3, D6, D10.
