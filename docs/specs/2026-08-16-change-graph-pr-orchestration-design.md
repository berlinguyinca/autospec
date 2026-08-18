# AutoSpec Change Graph & PR Orchestration — design

**Status:** Proposed — novel core decomposable after its dependencies land; integration sections delegate (see below)  
**Target:** AutoSpec  
**Implementation Language:** Rust  
**Priority:** High / Foundational  
**Spec Type:** Architecture + Execution + GitHub Integration  
**Date:** 2026-08-16  
**Related:**
- `docs/specs/2026-08-16-autonomous-engineering-organization-design.md` (AS-AEO-001)
- `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
- `docs/specs/2026-08-16-multi-model-engineering-team-design.md`
- `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md`

## What this specification uniquely contributes

Verified against the tree: there is **no** ChangeSet, Change Graph, or stacked-PR
surface anywhere in `crates/`. This document's core is genuinely new and unbuilt:

- The Change Graph and ChangeSet abstractions (§5.3–§5.4, §5.7)
- Dependency discovery, dependency types and edge strength (§8)
- Stack selection policy — prefer independent PRs, stack only on real dependency (§9)
- Stack and ChangeSet size budgets (§10–§11)
- The PR contract (§12)
- Conflict-aware scheduling (§16)
- Three CI levels and CI optimization (§33–§34)
- Merge strategy, merge queue, bottom-up stack merge (§40–§42)
- Failure recovery and change-graph replanning (§43–§47)

That core is the reason to implement this spec. Everything below is integration
surface that already has an owner.

## Sections that DELEGATE — do not reimplement

Several sections restate subsystems specified elsewhere. Where this document and
the owner disagree, **the owner wins**, and this spec consumes it.

| Sections here | Owner | Note |
|---|---|---|
| §25 worktree ownership, §26 Docker labels, §27 temp resources, §28 deterministic cleanup, §29 cleanup after merge, §30 cleanup failure, §31 garbage collection | `2026-08-16-resource-lifecycle-cleanup-design.md` (ADR **D7**) | Both define `autospec cleanup --dry-run` and an `autospec.*` Docker label set. The resource spec is authoritative — it carries the lease/janitor/reconciliation model and eight safety invariants. **A ChangeSet is an owner identity in the resource ledger**, not a second cleanup engine. |
| §17 model role architecture, §18 separation of duties, §19 model assignment, §20 usage-aware fallback | AS-AEO-001 Epics 5–6 (ADR **D1**, **D2**) | Roles are the 14 snake_case set fixed by D2. The router is Epic 5; this spec supplies the *task shape* to route, not a routing engine. |
| §21 GitHub issue projection, §22 GitHub Project/Kanban, §23 stack visualization | AS-AEO-001 Epic 8 | Projects V2 is greenfield and must be built once. §23 stack visualization is the one genuinely additive piece and belongs to whoever builds Epic 8. |
| §37 test planning, §38 UI/UX validation, §39 documentation | AS-AEO-001 Epic 7 / Epic 9, and `2026-08-16-vision-image-generation-qualification-design.md` | Gate definitions live there; this spec declares which ChangeSets require which gate. |
| §48 cross-repository changes, §49 repository capability abstraction | Existing fleet layer | Fleet owns node/repository placement; per-repo execution stays here. |

## Existing code these sections must extend, not recreate

- **§14 DAG validation** — `crates/autospec-core/src/graph/order.rs` (117 lines) already
  performs topological ordering with cycle detection. Extend it.
- **§15.1 Ready queue** — `crates/autospec-core/src/coordination/ready_queue.rs` already
  exists, alongside `coordination/conductor.rs`. The Change Graph feeds that queue; it
  does not introduce a second one.
- **§13 Rust data model** — per AS-AEO-001 §66.1 and ADR **D5/D7**, ChangeSet persistence
  belongs in the shared database, not a new store.

## Decomposition gate

The novel core consumes three things that do not exist yet: the executor abstraction
(#3172/#3173), the resource ledger (resource-lifecycle Phase 1), and the router
(AS-AEO-001 Epic 5). Decomposing it before those land would produce issues that
cannot start, and would re-file the cleanup and routing duplication this header
exists to prevent.

**Decompose the novel core once the executor chain and resource-lifecycle Phase 1
have merged.** The delegated sections are never decomposed from this document.

---

## 1. Executive Summary

AutoSpec must move beyond treating a large issue as either:

1. one large pull request, or
2. one long linear stack of pull requests.

Instead, AutoSpec SHALL introduce a first-class **Change Graph** abstraction.

A Change Graph is a dependency-aware directed acyclic graph (DAG) describing how a specification or epic is decomposed into independently testable and mergeable units of change.

Each unit of change may become:

- a single pull request,
- a shallow stacked PR sequence,
- one of several parallel pull requests,
- a cross-repository dependency,
- or a non-code work item such as documentation, benchmarks, UI validation, or integration testing.

The guiding rule is:

> **A PR stack is an implementation detail of the Change Graph, not the unit of work.**

AutoSpec SHALL prefer independent mergeable pull requests where possible and use stacked pull requests only where a real code or schema dependency exists.

This feature SHALL integrate with:

- specification decomposition,
- issue/sub-issue generation,
- GitHub Projects/Kanban visualization,
- model assignment,
- planner/implementer/reviewer separation,
- local worktrees,
- Docker resources,
- CI,
- merge queues,
- post-merge validation,
- deterministic cleanup,
- retries,
- failure recovery,
- telemetry,
- benchmark/advisory systems,
- and AutoSpec's model-routing engine.

---

# 2. Motivation

Large AutoSpec work items currently risk one or more of the following failure modes:

- oversized issues,
- oversized PRs,
- long-running branches,
- unnecessary merge conflicts,
- difficult reviews,
- poor parallelism,
- brittle PR stacks,
- unclear dependencies,
- abandoned worktrees,
- abandoned Docker images/containers,
- orphaned local branches,
- stale remote branches,
- repeated validation of unrelated changes,
- models working on dependent tasks in the wrong order,
- the same model implementing and reviewing the same work,
- and poor visibility into overall implementation progress.

Stacked pull requests solve part of the problem, but making every large task a single stack creates a different form of coupling.

For example:

```text
PR1
 ↓
PR2
 ↓
PR3
 ↓
PR4
 ↓
PR5
 ↓
PR6
```

If PR2 changes, all downstream PRs may need rebasing or revalidation.

A better representation is the actual dependency graph:

```text
        ┌── PR C
PR A ─ PR B ─ PR D
        └── PR E

PR F
PR G
```

Only PR A → PR B is necessarily stacked.

PR C, D, and E can potentially execute in parallel after B.

PR F and G may be fully independent.

AutoSpec therefore needs a graph-based planning and delivery engine.

---

# 3. Goals

AutoSpec SHALL:

1. Decompose large specs/issues into dependency-aware Change Graphs.
2. Minimize unnecessary PR stack depth.
3. Maximize safe parallel implementation.
4. Produce small, coherent, independently reviewable PRs.
5. Support true stacked PRs where dependencies require them.
6. Support cross-repository dependencies.
7. Project Change Graph state into GitHub Issues and GitHub Projects.
8. Make dependency relationships explicit and machine-readable.
9. Track every execution resource back to a ChangeSet.
10. Deterministically clean up worktrees, branches, containers, images, temporary databases, and artifacts.
11. Enforce implementation/review separation of duties.
12. Route work based on model capability, benchmark score, usage availability, cost, and task type.
13. Support planner, implementer, reviewer, test planner, documentation writer, UI/UX reviewer, and orchestrator roles.
14. Validate PRs at PR, stack, and final merge/integration levels.
15. Recover cleanly from failed models, failed CI, merge conflicts, stale branches, and partial execution.
16. Provide a persistent audit trail for why the graph was decomposed and scheduled as it was.

---

# 4. Non-Goals

This feature SHALL NOT:

- force every task into multiple PRs,
- force every dependent change into a GitHub-native stack,
- make GitHub the source of truth for AutoSpec orchestration,
- allow arbitrary infinite PR decomposition,
- allow circular Change Graph dependencies,
- allow the same model instance to implement and approve the same ChangeSet,
- create hidden execution resources without an owner,
- merge dependent PRs in an invalid order,
- or bypass required CI/review gates.

---

# 5. Core Concepts

## 5.1 Spec

A human- or machine-authored implementation request.

Example:

```text
Add local LLM benchmarking, workload-aware model scoring,
automatic routing, UI reporting, and documentation.
```

---

## 5.2 Epic

A high-level implementation container created from a large spec.

An Epic can contain multiple ChangeSets and multiple repositories.

---

## 5.3 Change Graph

A directed acyclic graph:

```text
G = (V, E)
```

where:

- `V` = ChangeSets
- `E` = dependency edges

An edge:

```text
A -> B
```

means:

> B cannot be implemented, validated, or merged until the required state of A has been reached.

The exact required state SHALL be configurable per edge.

Possible dependency thresholds:

- planned
- implemented
- PR opened
- reviewed
- merged
- post-merge validated

Default:

```text
merged
```

unless a stacked PR relationship explicitly allows implementation against an unmerged parent.

---

## 5.4 ChangeSet

A ChangeSet is the smallest logical unit AutoSpec intends to deliver and validate independently.

A ChangeSet SHOULD represent exactly one conceptual change.

Examples:

- introduce benchmark result data model,
- implement hardware discovery,
- add CLI command,
- add model scoring,
- add model router,
- add UI panel,
- add documentation,
- add end-to-end test coverage.

A ChangeSet MAY produce zero, one, or multiple PRs, but SHOULD normally produce one PR.

---

## 5.5 Work Item

A concrete executable task inside a ChangeSet.

Examples:

- write Rust implementation,
- create migration,
- add tests,
- update docs,
- run UI automation,
- run benchmark,
- review code.

---

## 5.6 Pull Request

A GitHub pull request produced by a ChangeSet.

A PR has an AutoSpec ownership relationship:

```text
Epic
  └── ChangeSet
       └── PullRequest
```

---

## 5.7 PR Stack

A sequence of PRs where later branches depend directly on earlier unmerged branches.

Example:

```text
main
  ↓
PR A
  ↓
PR B
  ↓
PR C
```

AutoSpec SHALL use a stack only when the dependency cannot reasonably be represented as independent PRs against `main`.

---

## 5.8 Execution

An Execution is one attempt by one worker/model to perform one Work Item.

Each Execution SHALL have a unique ID.

All execution resources SHALL be tagged with that ID and/or ChangeSet ID.

---

# 6. Required Hierarchy

AutoSpec SHALL implement the following logical hierarchy:

```text
SPEC
 ↓
EPIC
 ↓
CHANGE GRAPH
 ↓
CHANGE SET
 ↓
WORK ITEM
 ↓
EXECUTION
 ↓
PR / PR STACK
 ↓
REVIEW
 ↓
CI / VALIDATION
 ↓
MERGE QUEUE
 ↓
POST-MERGE VALIDATION
 ↓
CLEANUP
```

---

# 7. Example Decomposition

Given:

> Add local model benchmarking, advisory scoring, routing, UI reporting, hardware detection, and documentation.

AutoSpec might generate:

```text
EPIC #500
│
├── CS-501 Benchmark result data model
├── CS-502 Benchmark runner
├── CS-503 Hardware discovery
├── CS-504 Model capability scoring
├── CS-505 Advisory engine
├── CS-506 Router integration
├── CS-507 CLI integration
├── CS-508 UI
├── CS-509 Documentation
└── CS-510 Acceptance tests
```

Dependencies:

```text
CS-501
   │
   ├── CS-502
   │      │
   │      └── CS-504
   │             │
   │             └── CS-505
   │                    │
   │                    └── CS-506
   │
   └── CS-507

CS-503 ────────────────┐
                       │
CS-508 ────────────────┼── CS-510
                       │
CS-509 ────────────────┘
```

AutoSpec SHALL schedule ready nodes in parallel.

---

# 8. Graph Planning Rules

## 8.1 Dependency Discovery

The planner SHALL identify dependencies based on:

- APIs,
- traits,
- schema,
- database migrations,
- shared Rust types,
- generated code,
- feature flags,
- build dependencies,
- test fixtures,
- UI/backend contracts,
- CLI interfaces,
- external services,
- repository dependencies,
- deployment order,
- documentation dependencies,
- and acceptance-test prerequisites.

---

## 8.2 Dependency Types

AutoSpec SHALL support at minimum:

```rust
enum DependencyType {
    Code,
    Api,
    Schema,
    Database,
    Build,
    Runtime,
    Test,
    Documentation,
    Deployment,
    Repository,
    Artifact,
}
```

---

## 8.3 Edge Strength

Dependencies SHALL include a strength:

```rust
enum DependencyStrength {
    Hard,
    Soft,
    Advisory,
}
```

### Hard

The child cannot safely proceed.

### Soft

The child can start, but some final work or validation depends on the parent.

### Advisory

Scheduler should consider the relationship, but it is not blocking.

---

# 9. Stack Selection Policy

AutoSpec SHALL NOT convert all dependency chains into stacks.

Instead, the planner SHALL decide whether a dependency becomes:

1. independent PRs,
2. a stacked PR relationship,
3. delayed implementation,
4. a shared preparatory PR,
5. or a larger combined ChangeSet.

## 9.1 Prefer Independent PRs

Default policy:

> Prefer a PR based on the target branch over another stack layer.

A child SHOULD branch from `main` or the configured target branch when it can compile, test, and be reviewed independently.

---

## 9.2 Use a Stack When

A stacked PR is appropriate when:

- child code requires unmerged parent code to compile,
- an interface must be introduced before implementation,
- schema migration and follow-up code are intentionally separated,
- review size would be significantly reduced,
- or the parent must remain independently reviewable while child work continues.

---

## 9.3 Do Not Stack Merely Because

AutoSpec SHALL NOT stack PRs solely because:

- they belong to the same epic,
- they touch the same subsystem,
- they were assigned to the same model,
- they were created at the same time,
- or they have a conceptual relationship without a code dependency.

---

# 10. Stack Budgets

AutoSpec SHALL implement configurable stack budgets.

Recommended defaults:

```toml
[change_graph.stacks]
preferred_depth = 2
max_depth = 4
prefer_independent_prs = true
```

The scheduler/planner SHOULD actively attempt to flatten deeper stacks.

Example:

Instead of:

```text
A -> B -> C -> D -> E
```

prefer:

```text
        ┌-> C
A -> B -+-> D
        └-> E
```

when architectural boundaries permit it.

If `max_depth` would be exceeded, AutoSpec SHALL:

1. attempt graph restructuring,
2. attempt interface extraction,
3. attempt combining tightly coupled nodes,
4. or require a planner escalation.

It SHALL NOT silently create an arbitrarily deep stack.

---

# 11. ChangeSet Size Budgets

Recommended configurable policies:

```toml
[change_graph.change_sets]
max_primary_concepts = 1
max_high_risk_changes = 1
target_review_minutes = 15
target_diff_lines = 400
warning_diff_lines = 800
```

These are heuristics, not absolute hard limits.

AutoSpec SHALL prioritize conceptual coherence over raw line counts.

---

# 12. PR Contract

Every AutoSpec-generated PR SHALL contain or reference a machine-readable PR contract.

Recommended format:

```yaml
autospec:
  version: 1

  epic: EPIC-500
  change_set: CS-504

  purpose: model-capability-scoring

  repository: Berlinguyinca/autospec

  depends_on:
    - change_set: CS-502
      type: code
      strength: hard
      required_state: merged

  stack:
    enabled: true
    root: CS-502
    parent: CS-502
    position: 2

  provides:
    - CapabilityScore
    - ModelWorkloadProfile

  modifies:
    - src/advisory/**
    - src/models/**

  acceptance:
    - cargo-test
    - cargo-clippy
    - cargo-fmt
    - benchmark-fixtures

  risk:
    level: medium

  roles:
    planner: claude-opus
    implementer: codex
    reviewer: claude-opus

  cleanup:
    branch: true
    worktree: true
    containers: true
    images: conditional
    temp_databases: true
```

The exact provider/model names SHALL be dynamically selected, not hardcoded.

---

# 13. Rust Data Model

Suggested structures:

```rust
pub struct ChangeGraph {
    pub id: ChangeGraphId,
    pub epic_id: EpicId,
    pub nodes: HashMap<ChangeSetId, ChangeSet>,
    pub edges: Vec<DependencyEdge>,
    pub state: ChangeGraphState,
}

pub struct ChangeSet {
    pub id: ChangeSetId,
    pub title: String,
    pub description: String,
    pub repository: RepositoryRef,
    pub state: ChangeSetState,
    pub risk: RiskLevel,
    pub estimated_scope: ScopeEstimate,
    pub work_items: Vec<WorkItemId>,
    pub pull_requests: Vec<PullRequestRef>,
    pub execution_ids: Vec<ExecutionId>,
    pub resource_scope: ResourceScope,
}

pub struct DependencyEdge {
    pub from: ChangeSetId,
    pub to: ChangeSetId,
    pub dependency_type: DependencyType,
    pub strength: DependencyStrength,
    pub required_state: ChangeSetStateThreshold,
}

pub enum ChangeSetState {
    Proposed,
    Planned,
    Ready,
    Running,
    ImplementationComplete,
    PullRequestOpen,
    Reviewing,
    ChangesRequested,
    Approved,
    MergeQueued,
    Merged,
    PostMergeValidating,
    Complete,
    Blocked,
    Failed,
    Cancelled,
}
```

AutoSpec SHALL persist this state.

---

# 14. DAG Validation

Before execution, AutoSpec SHALL verify:

- no cycles,
- all referenced ChangeSets exist,
- no illegal cross-repository stack relationship,
- stack depths remain within policy,
- required repositories are available,
- required providers/models are available or have fallbacks,
- required CI capabilities exist,
- and required acceptance gates are resolvable.

Cycle detection SHALL fail planning.

Example error:

```text
ChangeGraphInvalid:
  cycle detected:
  CS-104 -> CS-109 -> CS-111 -> CS-104
```

---

# 15. Scheduling

## 15.1 Ready Queue

A ChangeSet becomes READY when all hard dependencies meet their required state.

The scheduler SHALL maintain a ready queue.

Example:

```text
READY:
- CS-503 Hardware discovery
- CS-508 UI scaffold
- CS-509 Documentation scaffold
```

---

## 15.2 Parallelism

Parallel execution SHALL be limited by:

- host CPU,
- RAM,
- GPU,
- disk,
- Docker capacity,
- repository conflicts,
- CI capacity,
- model/provider usage limits,
- API rate limits,
- cost policies,
- and configured worker limits.

Suggested configuration:

```toml
[scheduler]
max_parallel_change_sets = 6
max_parallel_per_repo = 4
max_parallel_gpu_jobs = 1
```

---

# 16. Conflict-Aware Scheduling

Two independent nodes MAY still conflict operationally.

AutoSpec SHOULD calculate a conflict score from:

- overlapping files,
- overlapping modules,
- migrations,
- Cargo.toml,
- Cargo.lock,
- generated files,
- shared configuration,
- shared test fixtures,
- UI routing files,
- database schema,
- and build scripts.

Example:

```text
CS-A and CS-B:
  dependency = none
  file_overlap = high
```

Scheduler MAY serialize them even without a graph dependency.

This SHALL be recorded as a scheduling constraint, not falsely added as a semantic dependency.

---

# 17. Model Role Architecture

AutoSpec SHALL support distinct roles:

- orchestrator,
- planner,
- implementer,
- reviewer,
- test planner,
- test reviewer,
- documentation writer,
- UI/UX reviewer,
- benchmark evaluator,
- security reviewer where required.

---

# 18. Separation of Duties

For each ChangeSet:

```text
implementer != reviewer
```

SHALL be enforced.

Additionally, when configured:

```text
implementer != planner
```

SHOULD be preferred for medium/high-risk work.

The same high-capability model MAY be used for planning and review if AutoSpec policy permits, but the implementation role SHALL remain separated.

---

# 19. Model Assignment

Model selection SHALL use AutoSpec's benchmark/advisory system.

Assignment SHALL consider:

- coding performance,
- reasoning performance,
- textual analysis performance,
- vision performance,
- UI interpretation performance,
- tool-use reliability,
- review quality,
- test generation,
- documentation quality,
- speed,
- cost,
- context window,
- provider quota,
- current availability,
- and task-specific benchmark score.

Example conceptual assignment:

```text
Planner:
  strongest reasoning/planning model available

Implementer:
  strongest coding model within policy

Reviewer:
  strong independent reasoning/coding reviewer

UI/UX Reviewer:
  vision-capable model

Documentation Writer:
  model with strong textual/documentation score
```

---

# 20. Usage-Aware Fallback

Before assigning any work, AutoSpec SHALL verify model/provider capacity.

If a preferred provider has insufficient quota/capacity:

```text
preferred model
      ↓ unavailable
fallback model
      ↓
next eligible model
```

Fallback SHALL preserve role separation.

Example:

If Codex implemented a ChangeSet, AutoSpec SHALL NOT use another Codex execution as its reviewer when the configured separation policy prohibits that model family/provider overlap.

---

# 21. GitHub Issue Projection

AutoSpec SHALL map Change Graph structure into GitHub.

Recommended mapping:

```text
Spec
  -> Epic Issue

ChangeSet
  -> Sub-Issue

DependencyEdge
  -> blocked-by / blocking relationship where supported

PullRequest
  -> linked implementation artifact

Graph State
  -> Project fields / labels / status
```

GitHub is a projection of the graph.

AutoSpec's persisted Change Graph SHALL remain the source of truth.

---

# 22. GitHub Project / Kanban

AutoSpec SHALL create or update a GitHub Project view for active Epics when project integration is enabled.

Recommended fields:

- Epic
- ChangeSet
- Status
- Repository
- Assigned Role
- Assigned Model
- Risk
- Dependency Count
- Blocked By
- PR
- Stack
- Stack Position
- CI
- Review
- Merge Queue
- Cleanup
- Last Activity

Suggested statuses:

```text
Planned
Ready
Running
PR Open
Review
Changes Requested
Approved
Merge Queued
Merged
Post-Merge Validation
Done
Blocked
Failed
```

---

# 23. Stack Visualization

AutoSpec SHOULD display stack relationships distinctly from ordinary dependencies.

Example:

```text
CS-101  [PR #700]
   │ STACK
CS-102  [PR #704]
   │ STACK
CS-103  [PR #709]
```

Parallel dependencies:

```text
          ┌── CS-104
CS-103 ───┼── CS-105
          └── CS-106
```

---

# 24. Branch Naming

Recommended convention:

```text
autospec/<epic>/<changeset>/<slug>
```

Example:

```text
autospec/500/504/model-capability-scoring
```

For a stacked child:

```text
autospec/500/505/advisory-engine
```

Stack state SHALL be stored in metadata rather than encoded only in the branch name.

---

# 25. Worktree Ownership

Every execution worktree SHALL have an explicit owner.

Example:

```text
.autospec/worktrees/
  EPIC-500/
    CS-504/
      EXEC-01/
```

Metadata SHALL include:

```yaml
owner:
  epic: EPIC-500
  change_set: CS-504
  execution: EXEC-01

branch: autospec/500/504/model-capability-scoring
created_at: ...
last_used_at: ...
state: active
```

No AutoSpec-created worktree SHALL exist without ownership metadata.

---

# 26. Docker Ownership

AutoSpec-created Docker resources SHALL include labels.

Required labels:

```text
autospec.managed=true
autospec.epic=EPIC-500
autospec.change_set=CS-504
autospec.execution=EXEC-01
autospec.repository=Berlinguyinca/autospec
```

This SHALL apply to:

- containers,
- images where feasible,
- networks,
- volumes,
- build cache metadata.

---

# 27. Temporary Resource Ownership

The ownership model SHALL also cover:

- temp databases,
- temp directories,
- downloaded fixtures,
- generated benchmark files,
- test servers,
- local ports,
- browser automation profiles,
- screenshots,
- logs,
- generated artifacts.

---

# 28. Deterministic Cleanup

When a ChangeSet reaches a terminal state, AutoSpec SHALL evaluate all owned resources.

Terminal states:

```text
Complete
Failed
Cancelled
```

Default cleanup:

```text
worktree             remove
local branch          remove if merged/cancelled and safe
remote branch         remove after merge if configured
running containers    stop/remove
temporary containers remove
temporary networks    remove
temporary volumes     remove if disposable
temporary DB          remove
temporary artifacts   remove according to retention
logs                  retain according to policy
benchmark outputs     retain if declared result artifact
```

---

# 29. Cleanup After Merge

Example lifecycle:

```text
PR merged
   ↓
post-merge validation
   ↓
ChangeSet complete
   ↓
resource inventory
   ↓
cleanup plan
   ↓
cleanup execution
   ↓
cleanup verification
   ↓
resource ownership record closed
```

AutoSpec SHALL verify cleanup rather than merely attempt it.

---

# 30. Cleanup Failure

Cleanup failures SHALL NOT silently disappear.

Example:

```text
CleanupIncomplete:
  CS-504

remaining:
  docker-container autospec-cs504-test-db
```

The ChangeSet MAY be marked functionally complete but SHALL carry:

```text
cleanup_state = incomplete
```

until resolved.

---

# 31. Garbage Collection

AutoSpec SHALL provide a global garbage collector.

Suggested command:

```bash
autospec cleanup
```

Additional modes:

```bash
autospec cleanup --dry-run
autospec cleanup --epic EPIC-500
autospec cleanup --change-set CS-504
autospec cleanup --orphans
autospec cleanup --older-than 24h
```

The garbage collector SHALL only remove resources:

- explicitly owned by AutoSpec,
- provably stale,
- or approved by configured safety policy.

---

# 32. PR CI

Each PR SHALL run targeted validation.

Minimum Rust checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Where repository policy differs, AutoSpec SHALL respect repository-specific configuration.

---

# 33. Three CI Levels

AutoSpec SHALL support:

## Level 1 — PR CI

Validates the individual ChangeSet.

Typical:

- compile,
- unit tests,
- lint,
- formatting,
- targeted subsystem tests.

---

## Level 2 — Stack CI

Validates the cumulative stack tip.

Typical:

- integration tests,
- stack-aware compile,
- API/schema compatibility,
- affected subsystem tests.

---

## Level 3 — Merge / Integration CI

Validates the proposed final merge against the latest target branch.

Typical:

- full required CI,
- merge queue validation,
- cross-ChangeSet integration,
- end-to-end tests,
- benchmark regression gates where applicable.

---

# 34. CI Optimization

AutoSpec SHOULD avoid rerunning the entire repository suite for every graph node when safely avoidable.

It SHOULD determine affected tests based on:

- touched packages,
- dependency graph,
- historical test impact,
- declared PR contract,
- Cargo workspace membership,
- benchmark dependencies,
- UI/backend boundaries.

Full validation remains mandatory at configured integration gates.

---

# 35. Review

Every PR SHALL receive an independent review unless explicitly exempted by policy.

Review SHALL validate:

- correctness,
- scope,
- architecture,
- tests,
- acceptance criteria,
- compatibility,
- security where applicable,
- regressions,
- code quality,
- documentation impact,
- and graph contract compliance.

---

# 36. Stack-Aware Review

Reviewers SHALL focus on the delta belonging to the current stack layer.

AutoSpec SHALL avoid presenting the reviewer with unrelated cumulative changes where possible.

Each stack layer SHALL remain understandable as its own conceptual change.

---

# 37. Test Planning

Complex ChangeSets SHOULD receive a test plan before implementation.

The test planner SHOULD be independent from the implementer for medium/high-risk work.

Test plan outputs MAY include:

- unit tests,
- integration tests,
- regression tests,
- property tests,
- benchmark tests,
- UI automation,
- failure injection,
- migration tests,
- backward compatibility tests.

---

# 38. UI/UX Validation

UI-related ChangeSets SHALL support a vision-capable review workflow.

Suggested flow:

```text
implementation
 ↓
browser automation
 ↓
screenshots / recordings
 ↓
vision model review
 ↓
UX findings
 ↓
fixes
 ↓
revalidation
```

AutoSpec SHALL link visual artifacts to the ChangeSet.

---

# 39. Documentation

Documentation SHALL be treated as a graph node where appropriate.

Documentation may execute in parallel once interfaces stabilize.

The documentation writer SHALL receive:

- final public API,
- CLI behavior,
- configuration,
- examples,
- migration notes,
- acceptance criteria.

---

# 40. Merge Strategy

A ChangeSet SHALL become merge-eligible when:

- implementation complete,
- required tests pass,
- required review approved,
- hard parent dependencies satisfy policy,
- stack ordering valid,
- merge conflicts resolved,
- required GitHub checks green.

---

# 41. Merge Queue

Where repository policy supports it, AutoSpec SHOULD submit merge-ready PRs to the merge queue.

AutoSpec SHALL NOT assume that a green PR branch remains green against a changing target branch.

Merge queue state SHALL be reflected in ChangeSet state.

---

# 42. Bottom-Up Stack Merge

Stacked PRs SHALL normally merge bottom-up.

Example:

```text
PR A
 ↓
PR B
 ↓
PR C
```

Merge order:

```text
A
B
C
```

After a parent merges, AutoSpec SHALL:

- update/rebase child base,
- rerun required stack validation,
- resolve trivial mechanical updates automatically,
- escalate semantic conflicts.

---

# 43. Failure Recovery

AutoSpec SHALL persist enough state to resume after:

- process crash,
- host reboot,
- model timeout,
- provider outage,
- GitHub failure,
- CI failure,
- Docker failure,
- merge conflict,
- network interruption.

No critical orchestration state SHALL exist only in process memory.

---

# 44. Model Failure

If an implementation model fails:

```text
Execution failed
   ↓
record failure
   ↓
preserve useful artifacts
   ↓
cleanup abandoned runtime resources
   ↓
select eligible fallback model
   ↓
resume from ChangeSet state
```

The replacement model SHALL receive a concise handoff rather than blindly restarting when reusable work exists.

---

# 45. CI Failure

CI failure classification SHOULD include:

```rust
enum CiFailureType {
    Compile,
    Test,
    Lint,
    Formatting,
    Environment,
    Flaky,
    Dependency,
    Infrastructure,
    Unknown,
}
```

AutoSpec SHOULD route failures to the appropriate remediation model/workflow.

---

# 46. Merge Conflict

AutoSpec SHALL distinguish:

- mechanical conflict,
- semantic conflict.

Mechanical conflict MAY be auto-resolved.

Semantic conflict SHALL trigger replanning/review.

If a conflict implies the Change Graph is no longer valid, affected nodes SHALL return to planning.

---

# 47. Change Graph Replanning

AutoSpec SHALL support graph mutation during execution.

Examples:

- split a ChangeSet,
- combine two ChangeSets,
- insert a newly discovered prerequisite,
- remove obsolete work,
- add a regression-fix node,
- change an edge from soft to hard.

All mutations SHALL be logged.

---

# 48. Cross-Repository Changes

AutoSpec SHALL support one Epic spanning multiple repositories.

Example:

```text
repo-contracts / PR A
        ↓
repo-backend / PR B
        ↓
repo-ui / PR C
```

Cross-repository dependencies SHALL exist in AutoSpec's graph even where GitHub-native stacked PR semantics do not apply.

---

# 49. Repository Capability Abstraction

AutoSpec SHALL NOT hard-wire orchestration to one GitHub PR-stack implementation.

Introduce a capability layer such as:

```rust
pub trait RepositoryProvider {
    fn supports_sub_issues(&self) -> bool;
    fn supports_issue_dependencies(&self) -> bool;
    fn supports_native_pr_stacks(&self) -> bool;
    fn supports_merge_queue(&self) -> bool;
    fn supports_projects(&self) -> bool;
}
```

This allows graceful fallback as GitHub features evolve.

---

# 50. GitHub Fallback Behavior

If native stack support is unavailable, AutoSpec SHALL still support stacks using standard branch bases.

If issue dependency support is unavailable, AutoSpec SHALL represent dependencies using:

- metadata,
- comments,
- labels,
- or project fields.

The internal Change Graph remains authoritative.

---

# 51. Orchestrator Responsibilities

The orchestrator SHALL:

1. load the Change Graph,
2. validate it,
3. discover ready ChangeSets,
4. evaluate scheduling constraints,
5. assign models,
6. create isolated execution environments,
7. launch workers,
8. monitor executions,
9. collect results,
10. open/update PRs,
11. trigger review,
12. trigger CI,
13. queue merge,
14. run post-merge validation,
15. clean resources,
16. update GitHub projection,
17. emit telemetry,
18. and replan where necessary.

---

# 52. Planner Responsibilities

The planner SHALL:

- decompose work,
- identify dependencies,
- identify opportunities for parallelism,
- decide PR boundaries,
- identify stack candidates,
- estimate risk,
- create acceptance criteria,
- identify testing needs,
- identify documentation needs,
- identify UI/UX validation needs.

The planner SHALL explain why each dependency exists.

---

# 53. Planner Output

Planner output SHALL be machine-readable.

Example:

```yaml
change_sets:
  - id: CS-501
    title: Benchmark result data model
    repository: Berlinguyinca/autospec
    risk: medium

  - id: CS-502
    title: Benchmark runner
    repository: Berlinguyinca/autospec
    risk: medium

edges:
  - from: CS-501
    to: CS-502
    type: code
    strength: hard
    reason: >
      Benchmark runner depends on the result model
      introduced by CS-501.
```

---

# 54. Scheduling Score

AutoSpec SHOULD rank READY ChangeSets.

Possible score inputs:

```text
critical-path priority
+ unblock count
+ user priority
+ merge value
+ model availability
+ resource fit
- conflict probability
- execution cost
- risk
```

This allows AutoSpec to prioritize work that unlocks the most downstream work.

---

# 55. Critical Path

AutoSpec SHOULD calculate the Change Graph critical path.

Nodes on the critical path SHOULD receive elevated scheduling priority when safe.

Example:

```text
A -> B -> C -> D
     \
      E

Critical path:
A -> B -> C -> D
```

---

# 56. Graph Metrics

AutoSpec SHALL expose:

- node count,
- edge count,
- ready nodes,
- blocked nodes,
- running nodes,
- completed nodes,
- critical path length,
- max stack depth,
- average stack depth,
- parallelism factor,
- merge throughput,
- review latency,
- CI latency,
- cleanup success rate,
- retry count,
- model failure count.

---

# 57. PR Metrics

Track:

- lines changed,
- files changed,
- review rounds,
- comments,
- CI attempts,
- merge conflicts,
- time to first review,
- time to merge,
- regression count,
- post-merge failures.

These metrics SHOULD feed future planning heuristics.

---

# 58. Model Performance Feedback

Completed ChangeSets SHOULD feed AutoSpec's benchmark/advisory system.

For each role/model combination record:

- completion success,
- review rejection rate,
- code defect rate,
- CI repair count,
- implementation time,
- token usage,
- cost,
- regression rate,
- human intervention,
- cleanup reliability.

This allows real AutoSpec work to continuously improve routing decisions.

---

# 59. CLI

Recommended commands:

```bash
autospec graph plan <spec>
autospec graph show <epic>
autospec graph validate <epic>
autospec graph run <epic>
autospec graph pause <epic>
autospec graph resume <epic>
autospec graph replan <epic>
```

ChangeSet commands:

```bash
autospec changeset show CS-504
autospec changeset retry CS-504
autospec changeset cancel CS-504
```

PR commands:

```bash
autospec pr status CS-504
autospec pr stack show EPIC-500
```

Cleanup:

```bash
autospec cleanup --dry-run
autospec cleanup --epic EPIC-500
autospec cleanup --orphans
```

---

# 60. Human Controls

Users SHALL be able to:

- approve the full graph,
- approve only high-risk nodes,
- pause an Epic,
- pause a ChangeSet,
- reassign a model,
- force serial execution,
- alter stack depth,
- force independent PR,
- combine nodes,
- split nodes,
- block merge,
- retry,
- cancel.

---

# 61. Autonomy Levels

This feature SHOULD integrate with AutoSpec autonomy levels.

Example:

```text
Level 1:
  suggest graph only

Level 2:
  create issues/branches with approval

Level 3:
  implement/open PRs automatically

Level 4:
  review/fix automatically, human merge

Level 5:
  merge approved low-risk work automatically

Level 6+:
  autonomously replan, schedule, merge,
  validate, and clean within policy
```

Exact names SHALL align with AutoSpec's existing autonomy model.

---

# 62. Risk Policy

Suggested risk levels:

```rust
enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}
```

High/Critical work MAY require:

- stronger planner,
- stronger reviewer,
- human approval,
- more extensive CI,
- lower parallelism,
- no auto-merge.

---

# 63. Audit Log

AutoSpec SHALL record important decisions.

Examples:

```text
Why was CS-504 stacked on CS-502?
Why was CS-508 executed in parallel?
Why was Codex selected?
Why did Claude review it?
Why was a retry triggered?
Why was a branch deleted?
Why was the graph replanned?
```

This SHALL be queryable.

---

# 64. Persistent Storage

The Change Graph SHALL be stored persistently.

Possible representation:

```text
.autospec/
  epics/
    EPIC-500/
      graph.json
      events.jsonl
      changesets/
      executions/
      artifacts/
```

A database-backed implementation MAY replace or supplement this later.

---

# 65. Event Model

Recommended internal events:

```rust
enum ChangeGraphEvent {
    EpicCreated,
    GraphPlanned,
    GraphValidated,
    ChangeSetReady,
    ExecutionStarted,
    ExecutionCompleted,
    ExecutionFailed,
    PullRequestOpened,
    ReviewStarted,
    ReviewApproved,
    ReviewChangesRequested,
    CiStarted,
    CiPassed,
    CiFailed,
    MergeQueued,
    PullRequestMerged,
    PostMergeValidationStarted,
    PostMergeValidationPassed,
    CleanupStarted,
    CleanupCompleted,
    CleanupFailed,
    GraphReplanned,
}
```

An event log enables reconstruction and debugging.

---

# 66. Security

AutoSpec SHALL:

- avoid exposing secrets in PR contracts,
- scope GitHub credentials minimally,
- isolate model execution environments,
- avoid allowing arbitrary cleanup outside owned resources,
- validate paths before deletion,
- protect `main`/default branches,
- respect protected branch rules,
- respect required reviews,
- and never bypass security checks through orchestration.

---

# 67. Cleanup Safety

Cleanup SHALL be ownership-based.

AutoSpec MUST NOT delete a resource merely because its name resembles an AutoSpec resource.

Safe removal requires:

- ownership metadata,
- resource identity match,
- lifecycle state permitting cleanup.

---

# 68. Idempotency

All orchestration operations SHOULD be idempotent.

Examples:

```text
open PR
create issue
create worktree
stop container
remove container
mark state
submit merge queue
cleanup
```

Repeated execution after a crash SHALL not corrupt state.

---

# 69. Concurrency Safety

AutoSpec SHALL prevent two orchestrator processes from controlling the same ChangeSet simultaneously unless explicitly operating in distributed coordinated mode.

Use:

- locks,
- leases,
- compare-and-swap state,
- or equivalent mechanisms.

---

# 70. Initial Implementation Phases

## Phase 1 — Core Change Graph

Implement:

- Rust domain model,
- persistence,
- DAG validation,
- dependency states,
- ready queue,
- event log.

No GitHub automation required initially.

---

## Phase 2 — Planner Decomposition

Implement:

- structured planner output,
- dependency detection,
- ChangeSet generation,
- PR boundary heuristics,
- stack budget rules,
- graph validation.

---

## Phase 3 — Execution Ownership

Implement:

- Execution model,
- branch ownership,
- worktree ownership,
- resource labels,
- deterministic local cleanup.

---

## Phase 4 — GitHub Projection

Implement:

- Epic issue,
- sub-issues,
- dependency projection,
- PR linking,
- project status updates,
- labels/fields.

---

## Phase 5 — PR and Stack Execution

Implement:

- independent branch workflow,
- stacked branch workflow,
- stack state,
- parent rebasing,
- stack validation,
- bottom-up merge handling.

---

## Phase 6 — Model Routing

Integrate:

- planner role,
- implementer role,
- reviewer role,
- test planner,
- docs writer,
- UI reviewer,
- usage-aware fallback,
- separation of duties.

---

## Phase 7 — CI / Merge Queue

Implement:

- PR CI,
- stack CI,
- merge/integration CI,
- merge queue integration,
- post-merge validation.

---

## Phase 8 — Cleanup / GC

Implement:

- terminal cleanup,
- cleanup verification,
- orphan scanning,
- dry-run mode,
- retention policies,
- cleanup telemetry.

---

## Phase 9 — Replanning / Recovery

Implement:

- graph mutation,
- retry,
- model failover,
- CI diagnosis,
- conflict handling,
- crash recovery,
- stale execution recovery.

---

## Phase 10 — Metrics and Adaptive Planning

Implement:

- graph metrics,
- PR metrics,
- model performance feedback,
- planning heuristic updates,
- benchmark feedback loop.

---

# 71. Required Test Matrix

## 71.1 DAG Tests

Test:

- valid graph,
- isolated node,
- diamond graph,
- multiple roots,
- multiple leaves,
- cycle rejection,
- missing dependency rejection.

---

## 71.2 Stack Tests

Test:

- single PR,
- two-deep stack,
- max-depth stack,
- over-depth rejection/replanning,
- parent merge,
- child rebase,
- child conflict,
- stack CI.

---

## 71.3 Parallelism Tests

Test:

- independent nodes execute concurrently,
- dependency blocks execution,
- soft dependency behavior,
- conflict-aware serialization,
- resource-limited scheduling.

---

## 71.4 Model Assignment Tests

Test:

- preferred model available,
- preferred model unavailable,
- quota exhausted,
- fallback selected,
- reviewer separation enforced,
- unsupported role rejected.

---

## 71.5 Cleanup Tests

Test:

- successful merge cleanup,
- failed execution cleanup,
- cancelled ChangeSet cleanup,
- active resource not removed,
- unrelated resource not removed,
- orphan detection,
- cleanup restart after crash.

---

## 71.6 GitHub Tests

Test:

- Epic creation,
- sub-issue projection,
- dependencies,
- PR linking,
- state updates,
- stack metadata,
- project fields,
- merge queue,
- branch deletion.

---

## 71.7 Recovery Tests

Test interruption after every major event:

```text
after worktree creation
after implementation
after commit
after push
after PR creation
during review
during CI
during merge
during cleanup
```

AutoSpec SHALL recover without duplication or resource leaks.

---

# 72. Acceptance Criteria

This specification is considered implemented when all of the following are true.

## Graph

- [ ] AutoSpec can represent a spec as a persistent DAG of ChangeSets.
- [ ] Cycles are rejected.
- [ ] ChangeSets become ready based on dependency state.
- [ ] Independent ChangeSets can execute in parallel.

## PR Architecture

- [ ] AutoSpec can create an independent PR.
- [ ] AutoSpec can create a stacked PR.
- [ ] AutoSpec limits stack depth.
- [ ] AutoSpec prefers independent PRs when possible.
- [ ] AutoSpec supports one Epic across multiple repositories.

## GitHub

- [ ] Epic and ChangeSet relationships are visible in GitHub.
- [ ] PRs link back to ChangeSets.
- [ ] status is visible in GitHub Projects or equivalent.
- [ ] dependency state is projected where possible.

## Models

- [ ] planning and implementation roles are represented.
- [ ] implementer/reviewer separation is enforced.
- [ ] provider/model quota is checked before assignment.
- [ ] fallback models can be selected.
- [ ] role-specific benchmark/advisory scores influence routing.

## CI

- [ ] PR CI exists.
- [ ] stack validation exists.
- [ ] final merge/integration validation exists.
- [ ] merge queue integration is supported where available.

## Cleanup

- [ ] every worktree has ownership.
- [ ] every AutoSpec Docker resource has ownership.
- [ ] merged ChangeSets clean up resources.
- [ ] cancelled/failed executions clean disposable resources.
- [ ] orphan cleanup exists.
- [ ] cleanup has dry-run mode.
- [ ] cleanup is verified.

## Recovery

- [ ] orchestration survives process restart.
- [ ] failed model execution can retry with fallback.
- [ ] CI failures are persisted.
- [ ] stack merge conflicts can be recovered.
- [ ] cleanup resumes after interruption.

## Observability

- [ ] graph state can be inspected.
- [ ] critical path can be calculated.
- [ ] active/blocking ChangeSets can be listed.
- [ ] events are logged.
- [ ] model performance feedback can be recorded.

---

# 73. Example End-to-End Workflow

Input:

```text
autospec implement SPEC-123
```

Flow:

```text
Spec loaded
   ↓
Planner decomposes spec
   ↓
Change Graph created
   ↓
Graph validated
   ↓
GitHub Epic + sub-issues projected
   ↓
Ready queue calculated
   ↓
Models assigned
   ↓
Independent ChangeSets run in parallel
   ↓
Stacked children run against required parents
   ↓
PRs opened
   ↓
Independent reviewers assigned
   ↓
PR CI
   ↓
Stack CI
   ↓
Approved PRs enter merge queue
   ↓
Bottom-up stack merge
   ↓
Final integration validation
   ↓
ChangeSets marked complete
   ↓
Owned resources cleaned
   ↓
GitHub Project updated
   ↓
Metrics returned to advisory/benchmark system
   ↓
Epic complete
```

---

# 74. Architectural Rule Summary

AutoSpec SHALL follow these rules:

1. **The Change Graph is the source of truth.**
2. **A PR stack is an implementation detail, not the unit of work.**
3. **Prefer independently mergeable PRs over deeper stacks.**
4. **Keep stacks shallow.**
5. **Maximize safe parallelism.**
6. **Every resource has an owner.**
7. **Implementer and reviewer are separated.**
8. **Model selection is workload-aware and usage-aware.**
9. **GitHub visualizes the plan but does not define it.**
10. **CI occurs at PR, stack, and integration levels.**
11. **Merge occurs only when graph dependencies permit it.**
12. **Cleanup is deterministic and verified.**
13. **Execution can resume after failure.**
14. **Real project outcomes feed back into model benchmarking and advisory routing.**

---

# 75. Final Expected Outcome

After implementation, AutoSpec should be able to take a large request that would previously become one oversized issue or one fragile long-running PR and automatically transform it into an execution graph such as:

```text
Epic
│
├── Foundation PR
│      │
│      └── Core Engine PR
│              ├── CLI PR
│              ├── API PR
│              └── Router PR
│
├── Hardware Discovery PR
├── UI PR
├── Documentation PR
└── Final Acceptance Tests
```

AutoSpec will then:

- schedule independent work in parallel,
- stack only genuinely dependent PRs,
- assign suitable models per role,
- enforce review independence,
- run appropriate CI,
- merge in dependency-safe order,
- validate the integrated result,
- clean every execution resource,
- update GitHub visualization,
- and use the observed results to improve future model selection and planning.

This should become a foundational AutoSpec orchestration capability and serve as the delivery substrate for future large-scale autonomous implementation work.
