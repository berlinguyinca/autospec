# AutoSpec Parallel Decomposition & Fleet Saturation Specification

Status: Ready for implementation
Target repository: berlinguyinca/autospec
Primary component: autospec-define
Related components: autospec-split, autospec-run, issue linting, ready queue, project board dependency projection
Fleet target: 10–100 concurrently available implementation agents
Design objective: Maximize safe implementation concurrency while preserving architectural quality, correctness, testability, and low merge contention.

---

## 1. Executive Summary

AutoSpec already supports concurrent implementation through its ready queue, isolated worktrees, issue claims, and dependency-aware execution. The main remaining bottleneck is planning: `autospec-define` can generate technically valid issue trees that are unnecessarily serial.

This specification changes decomposition from:

> produce a correct ordered list of implementation issues

to:

> produce the widest safe dependency DAG that can efficiently saturate the currently available implementation fleet.

The planner MUST prefer independent sibling issues over serial parent/child chains whenever correctness does not require sequencing.

Hard dependencies become an expensive scheduling primitive. They MUST only exist when an issue cannot be implemented or independently verified against the current base branch without the predecessor.

The implementation introduces:

1. capacity-aware issue decomposition for 10–100 agents;
2. explicit concurrency and ownership metadata;
3. dependency justification and minimization;
4. deterministic DAG analysis;
5. serialization and conflict lint rules;
6. a concurrency-review pass before issue creation;
7. fleet-saturation scoring;
8. execution-wave projections;
9. adaptive issue granularity;
10. compatibility with the existing auto-implement ready queue.

No initial change to the core `autospec-run` claim protocol is required.

---

## 2. Problem Statement

AutoSpec's existing decomposition rules correctly emphasize small, independently understandable implementation issues.

However, issue decomposition may still create avoidable dependency chains because:

- issue order is sometimes treated as dependency order;
- "foundation" issues can become broad barriers;
- issue-size splits may introduce unnecessary `Depends on` edges;
- merge-conflict risk and semantic dependency are conflated;
- planner prompts optimize local issue quality more strongly than fleet utilization;
- issue ownership is not explicit enough to minimize write collisions;
- no deterministic metric reports whether a generated DAG is too serial;
- the planner does not adapt decomposition granularity to currently available agent capacity.

With 10–100 idle workers, a dependency graph such as:

```text
A -> B -> C -> D -> E
```

is unacceptable when the implementation can safely be represented as:

```text
       B
      /
A --- C
      \
       D

E independent
```

The goal is not parallelism at any cost. The goal is maximum safe throughput.

---

## 3. Goals

### 3.1 Primary Goals

AutoSpec MUST:

- maximize the number of implementation issues that can execute concurrently;
- support fleet sizes from 10 to 100 workers;
- adapt issue granularity to current or configured fleet capacity;
- minimize unnecessary hard dependency edges;
- minimize critical-path length;
- minimize overlapping write ownership;
- distinguish semantic dependencies from probable merge conflicts;
- keep issues independently testable;
- preserve architectural boundaries;
- preserve TDD and acceptance-test requirements;
- avoid "AI fragmentation" that produces hundreds of meaningless micro-issues;
- generate enough immediately ready work to materially saturate the fleet.

### 3.2 Secondary Goals

AutoSpec SHOULD:

- produce deterministic concurrency metrics;
- explain why every hard dependency exists;
- identify suspicious serialization;
- identify high fan-in barriers;
- identify shared-write hotspots;
- expose projected execution waves;
- allow orchestration layers to consume concurrency metadata later.

---

## 4. Non-Goals

This version does NOT:

- replace the existing ready queue;
- replace GitHub issues with another task system;
- permit multiple agents to edit the same worktree;
- treat issue parenthood as an execution dependency;
- automatically merge conflicting PRs;
- weaken tests or architecture checks to achieve higher concurrency;
- split every function or file into separate issues;
- require exactly one issue per agent;
- require all 100 agents to be occupied when the feature is genuinely smaller than 100 useful work packages.

---

## 5. Core Principle

### 5.1 Hard dependencies require proof

A hard dependency MUST be added only when the dependent issue cannot be correctly implemented or independently verified against the current base branch without the predecessor.

Valid hard-dependency reasons include:

- predecessor introduces a required public API;
- predecessor introduces a required type or interface;
- predecessor introduces a required schema;
- predecessor introduces a required database migration;
- predecessor introduces a required wire/protocol version;
- predecessor introduces a generated artifact consumed by the child;
- predecessor performs a structural migration that must precede child changes;
- child acceptance tests literally cannot run without predecessor output.

The following MUST NOT create a hard dependency by themselves:

- issue appears earlier in the spec;
- issue is described as "foundational";
- implementation order would be convenient;
- issues belong to the same epic;
- files are nearby;
- one issue is documentation;
- one issue is testing;
- conceptual relationship;
- expected merge conflicts;
- parent/child relationship;
- planner preference;
- "do this first" wording without technical evidence.

---

## 6. Fleet Capacity Model

### 6.1 Supported Capacity

The planner MUST support an effective implementation capacity:

```text
10 <= fleet_capacity <= 100
```

Values outside the range MAY be accepted, but planning behavior MUST be tuned and tested for 10–100.

### 6.2 Capacity Sources

Capacity precedence:

1. explicit invocation flag;
2. project configuration;
3. orchestrator-discovered capacity;
4. environment variable;
5. default.

Proposed sources:

```text
/autospec-define --agents 48 ...
autospec define --agents 48 ...
AUTOSPEC_AGENT_CAPACITY=48
```

Project config:

```yaml
planning:
  parallelism:
    target_agents: 32
```

Default:

```text
32
```

The default MUST NOT reduce correctness if fewer agents exist. It is a decomposition target only.

### 6.3 Dynamic Capacity

Future orchestrators MAY provide:

```json
{
  "available_agents": 47,
  "total_agents": 64,
  "reserved_agents": 8,
  "effective_implementation_capacity": 39
}
```

`autospec-define` MUST use:

```text
effective_capacity = clamp(effective_implementation_capacity, 10, 100)
```

when available.

---

## 7. Capacity-Aware Decomposition

The planner MUST NOT simply attempt to create `fleet_capacity` issues.

Instead, it MUST optimize for useful concurrency.

### 7.1 Target Initial Width

For issue count N and fleet capacity C:

```text
target_initial_width =
    min(
        C,
        ceil(N * 0.60)
    )
```

This is a target, not a correctness gate.

For large specs, the planner SHOULD initially expose enough root work to keep at least 60% of the available fleet busy when technically possible.

### 7.2 Minimum Useful Issue Size

A child issue SHOULD represent approximately:

- one coherent architectural responsibility;
- one independently testable change;
- one principal write ownership domain;
- normally 1–3 logical implementation units.

An issue MUST NOT be split solely to increase the issue count.

Bad:

```text
#1 add struct field
#2 add getter
#3 add setter
#4 update constructor
```

Good:

```text
#1 add session policy model and validation
#2 implement HTTP policy adapter
#3 implement CLI policy adapter
#4 add persistence integration
```

### 7.3 Granularity Bands

Recommended planning target by capacity:

| Effective capacity | Planning behavior |
| --- | --- |
| 10–19 | medium-grained independent issues |
| 20–39 | finer architectural/workstream splitting |
| 40–69 | aggressively separate adapters, validation, tooling, observability, docs, benchmarks when independently testable |
| 70–100 | maximize ownership partitions and independent workstreams, but never create trivial getter/setter or single-line issues |

The planner MUST preserve a lower bound on meaningful engineering work.

---

## 8. Decomposition Pipeline

Update Phase 3 into explicit subphases.

```text
Phase 3.0 Functional decomposition
Phase 3.1 Contract extraction
Phase 3.2 Ownership partitioning
Phase 3.3 Candidate dependency graph
Phase 3.4 Concurrency optimization
Phase 3.5 Existing issue classification
Phase 3.6 DAG validation and scoring
Phase 3.7 GitHub issue creation
```

If preserving existing numbering is important, the new subphases MAY be embedded internally without renaming user-facing Phase 3.5.

---

## 9. Phase 3.0 — Functional Decomposition

The planner identifies independently meaningful capabilities.

Candidate split dimensions:

contracts and schemas; core domain; adapters; persistence; migrations; REST/API; CLI; UI; observability; logging; metrics; compatibility; tooling; tests; fixtures; documentation; migration utilities; benchmarks; deployment; feature flags; rollback support.

Output MUST initially be dependency-neutral.

Do not infer ordering yet.

---

## 10. Phase 3.1 — Contract Extraction

Before assigning dependencies, identify shared contracts.

Examples: trait/interface; request schema; database schema; protobuf/OpenAPI schema; CLI contract; config schema; event envelope; shared test fixture format.

Each shared contract is classified:

```yaml
contract:
  state: existing | needs-change | new
  owner_issue: optional
  consumers:
    - issue-id
```

If a contract already exists and consumers can build against it:

```text
NO DEPENDENCY REQUIRED
```

If a minimal contract change enables many workers, the planner SHOULD isolate it into a small "unlock" issue.

Example:

```text
           +-> HTTP adapter
contract --+-> CLI adapter
           +-> UI adapter
           +-> metrics
           +-> compatibility layer
```

The unlock issue SHOULD be deliberately small to reduce time-to-fan-out.

---

## 11. Phase 3.2 — Ownership Partitioning

Every child issue MUST have explicit write ownership.

Proposed metadata:

```yaml
ownership:
  exclusive:
    - path: crates/router/src/http.rs
      symbols:
        - HttpRouter
        - route_request
  shared_read:
    - crates/router/src/contracts.rs
  generated:
    - none
```

For directory-oriented work:

```yaml
ownership:
  exclusive:
    - path: web/src/features/session/**
```

The planner SHOULD optimize for:

```text
one issue -> one primary write domain
```

### 11.1 Shared Writes

Shared writes are allowed but MUST be declared:

```yaml
ownership:
  shared_write:
    - path: crates/core/src/lib.rs
      reason: registration export only
```

Shared-write metadata MUST contribute to conflict-risk scoring.

Shared writes MUST NOT automatically become dependency edges.

---

## 12. Phase 3.3 — Candidate Dependency Graph

Each proposed dependency MUST contain a machine-readable reason.

Example:

```yaml
dependencies:
  hard:
    - issue: 123
      reason_code: consumes-new-interface
      artifact: RouterBackend
```

Reason codes:

```text
consumes-new-interface
consumes-new-type
consumes-new-schema
consumes-migration
consumes-generated-artifact
requires-structural-migration
requires-new-protocol
verification-requires-predecessor
external-prerequisite
```

Free-form explanations MAY accompany codes.

Unsupported reason codes MUST fail lint.

---

## 13. Phase 3.4 — Concurrency Optimization

A dedicated Tier A planning/review subagent MUST inspect the proposed graph.

Its only objective is to challenge unnecessary serialization.

Prompt contract:

```text
You are the AutoSpec concurrency optimizer.

Assume between 10 and 100 implementation agents are available.

Review the candidate issue DAG and maximize safe execution width.

For every dependency:
1. identify the concrete artifact consumed by the child;
2. prove the child cannot implement or verify against current base without it;
3. remove the edge if that proof fails.

Then:
- identify issues that can be siblings;
- isolate small contract-unlock issues when they fan out substantial work;
- reduce shared write ownership;
- shorten critical path;
- avoid meaningless micro-issues;
- preserve architecture, testing, and correctness.

Do not introduce a dependency merely to avoid probable merge conflicts.
Represent conflict risk separately.
```

This pass MUST return:

```json
{
  "removed_edges": [],
  "added_edges": [],
  "split_issues": [],
  "merged_issues": [],
  "ownership_changes": [],
  "rationale": []
}
```

---

## 14. Concurrency Metadata Contract

Each generated issue MUST include:

```markdown
## Concurrency

Parallel safe: yes

### Exclusive write ownership
- `crates/router/src/http.rs` — `HttpRouter`, `route_request`

### Shared read contracts
- `crates/router/src/contracts.rs` — `RouterBackend`

### Shared write surfaces
- none

### Conflict domains
- `router-http`

### Hard dependency justification
- none

### Expected parallel peers
- issue #124
- issue #125
- issue #126
```

Expected parallel peers is informative only.

It MUST NOT become authoritative scheduling state.

---

## 15. Machine-Readable Issue Metadata

Longer term, issue bodies SHOULD contain a fenced metadata block:

```yaml
autospec:
  concurrency:
    parallel_safe: true
    conflict_domains:
      - router-http

  dependencies:
    hard:
      - issue: 123
        reason_code: consumes-new-interface
        artifact: RouterBackend

  ownership:
    exclusive:
      - path: crates/router/src/http.rs
        symbols:
          - HttpRouter
    shared_read:
      - crates/router/src/contracts.rs
    shared_write: []
```

The existing Markdown `## Dependencies` section remains authoritative for backward compatibility in v1.

Generated metadata MUST agree with the Markdown section.

Mismatch MUST fail lint.

---

## 16. Dependency Section

Canonical body format remains:

```markdown
## Dependencies

none
```

or:

```markdown
## Dependencies

Depends on issue #123
```

Add a justification section:

```markdown
## Dependency justification

- #123 — consumes `RouterBackend`, which does not exist on the current base branch.
```

For none:

```markdown
## Dependency justification

none
```

---

## 17. Fix TOO_MANY_FILES Behavior

Existing guidance that effectively encourages:

```text
large issue -> split -> dependency edge
```

MUST be replaced.

New rule:

```text
TOO_MANY_FILES

Split the issue into independent sibling issues whenever their outputs are separable.

A size-based split MUST NOT introduce a dependency by itself.

Add Depends on only when one split issue consumes a concrete artifact,
schema, interface, migration, generated output, or verification capability
created by the other.
```

This rule MUST be updated consistently in:

- `skills/autospec-define/SKILL.md`
- `skills/autospec-define/codex/prompt.md`
- `skills/autospec-define/opencode/agent.md`
- corresponding `autospec-split` surfaces if generated separately
- `tests/goldens` for skill synchronization

---

## 18. Conflict Risk Is Not Dependency

Introduce a separate conflict model.

```yaml
conflicts:
  probability: medium
  surfaces:
    - crates/core/src/lib.rs
  mitigation:
    - keep export-only changes minimal
    - rebase before final validation
```

Conflict levels:

```text
none
low
medium
high
```

A high conflict score MAY affect dispatch ordering later.

It MUST NOT make an issue blocked.

---

## 19. DAG Analyzer

Add deterministic analysis.

Proposed command:

```bash
autospec graph analyze
```

or script-first implementation:

```bash
scripts/autospec-analyze-issue-dag.sh
```

Inputs: proposed issue JSON; issue drafts; or GitHub issue bodies.

Outputs:

```json
{
  "issue_count": 37,
  "hard_edge_count": 11,
  "root_count": 17,
  "leaf_count": 12,
  "critical_path_length": 4,
  "maximum_width": 22,
  "initial_width": 17,
  "average_wave_width": 12.3,
  "serialization_ratio": 0.108,
  "shared_write_hotspots": 3,
  "high_fan_in_nodes": [],
  "high_fan_out_nodes": [],
  "estimated_fleet_saturation": {
    "capacity": 32,
    "initial": 0.53,
    "peak": 0.69
  }
}
```

---

## 20. Graph Metrics

### 20.1 Initial Width

Number of issues with zero unresolved hard dependencies.

```text
initial_width = |roots|
```

### 20.2 Maximum Width

Maximum number of simultaneously dependency-ready nodes assuming immediate predecessor completion and ignoring resource conflicts.

### 20.3 Critical Path Length

Longest hard-dependency chain.

### 20.4 Serialization Ratio

```text
serialization_ratio =
    hard_edge_count /
    max(1, issue_count * (issue_count - 1) / 2)
```

Also expose a more intuitive execution score:

```text
critical_path_pressure =
    critical_path_length / issue_count
```

### 20.5 Fleet Saturation

For capacity C:

```text
initial_saturation = min(initial_width, C) / C
peak_saturation = min(maximum_width, C) / C
```

Do not punish small features that naturally contain fewer useful work packages than C.

---

## 21. Parallelization Score

Produce a 0–100 advisory score.

Suggested components:

```text
30% initial saturation
20% peak saturation
20% inverse critical-path pressure
15% low shared-write overlap
15% dependency justification quality
```

The score MUST be advisory. Correctness always wins.

Example output:

```text
Parallelization score: 86/100

Fleet capacity:        48
Issues:                57
Initial ready:         31
Peak ready:            44
Critical path:         5
Hard dependencies:     13
Shared-write hotspots: 2
Suspect dependencies:  1
```

---

## 22. DAG Lint Rules

Add stable lint codes.

**AS-DAG-001 UNJUSTIFIED_DEPENDENCY** — dependency has no recognized reason code or artifact.

**AS-DAG-002 ORDER_ONLY_DEPENDENCY** — dependency rationale describes ordering/convenience rather than a technical prerequisite. Examples: `"implement first"`, `"foundation"`, `"do before UI"`, `"easier if"`.

**AS-DAG-003 ARTIFICIAL_SERIALIZATION** — child references no artifact produced by predecessor and appears independently implementable. Initially warning-level unless deterministic proof is available.

**AS-DAG-004 EXCESSIVE_FAN_IN** — issue depends on more than a configurable threshold. Default `5`. Require explicit justification for each edge.

**AS-DAG-005 EXCESSIVE_CRITICAL_PATH** — for issue count >= 10: `critical_path_length > max(5, ceil(issue_count * 0.30))`. Warning.

**AS-DAG-006 LOW_INITIAL_WIDTH** — for issue count >= 10 and capacity >= 10: `initial_width < min(capacity, max(4, ceil(issue_count * 0.25)))`. Warning and force concurrency-review retry once.

**AS-DAG-007 SHARED_WRITE_HOTSPOT** — three or more root issues declare overlapping primary write ownership.

**AS-DAG-008 SPLIT_CREATED_ORDERING** — two sibling-sized issues are connected only because they were produced by splitting one oversized issue.

**AS-DAG-009 METADATA_DEPENDENCY_MISMATCH** — machine metadata and Markdown dependency section differ.

**AS-DAG-010 CYCLE** — fatal.

---

## 23. Automatic Concurrency Retry

After initial decomposition:

1. analyze DAG;
2. run lint;
3. if AS-DAG-006, AS-DAG-003, or score below threshold:
4. run one dedicated concurrency optimization retry;
5. re-analyze;
6. accept improved graph unless correctness validation fails.

Default threshold:

```text
parallelization_score < 65
```

Do NOT repeatedly regenerate indefinitely.

Maximum automatic optimization passes:

```text
1
```

A second pass MAY be allowed via configuration.

---

## 24. Execution Waves

The analyzer MUST generate projected waves.

```text
Wave 0 — 17 issues
#101 #102 #103 #104 #105 #106 #107 #108 #109
#110 #111 #112 #113 #114 #115 #116 #117

Wave 1 — 9 issues
#118 #119 #120 #121 #122 #123 #124 #125 #126

Wave 2 — 4 issues
#127 #128 #129 #130

Wave 3 — 1 issue
#131
```

Waves are diagnostic projections. They MUST NOT replace dynamic ready-queue scheduling.

---

## 25. Planner Output Summary

At the end of `autospec-define`, print:

```text
AutoSpec decomposition complete

Spec: docs/specs/...
Fleet target: 48 agents

Issues created:          57
Initially ready:         31
Maximum projected ready: 44
Critical path:           5
Hard dependencies:       13
Shared-write hotspots:   2
Parallelization score:   86/100

Projected initial utilization: 31/48 agents

Concurrency review:
- removed 8 unnecessary dependency edges
- split 2 broad ownership issues
- created 1 contract-unlock issue
- reduced critical path from 9 to 5
```

---

## 26. autospec-run Compatibility

No v1 scheduling rewrite is required. Existing semantics remain:

```text
auto-implement      = candidate/ready implementation queue
in-progress-by-bot  = active claim
## Dependencies     = hard readiness prerequisites
```

The wider DAG automatically creates more issues that the existing ready queue can claim.

Future `autospec-run` versions MAY use conflict domains, ownership overlap, estimated task size, model fit, or agent specialization. Out of scope for v1.

---

## 27. Suggested File-Level Changes

### 27.1 Core Skill

Modify `skills/autospec-define/SKILL.md`, `skills/autospec-define/codex/prompt.md`, `skills/autospec-define/opencode/agent.md`.

Add: fleet capacity detection; capacity-aware decomposition objective; dependency proof rules; ownership partitioning; concurrency optimization pass; concurrency issue section; DAG validation; summary metrics; revised TOO_MANY_FILES behavior.

### 27.2 autospec-split

Mirror all decomposition rules in `skills/autospec-split/**`. Existing-spec decomposition MUST behave identically to normal `autospec-define` Phase 3.

### 27.3 Issue Skeleton

Modify `scripts/gen-issue-skeleton.sh`. Add `## Concurrency` and `## Dependency justification`. Support structured concurrency input fields.

### 27.4 Issue Linting

Modify `scripts/lint-issue.sh` and `crates/autospec-core/src/lint/mod.rs`. Add validation for the concurrency section, dependency justification, malformed hard dependencies, and metadata mismatch.

### 27.5 DAG Analysis

Add `crates/autospec-core/src/graph/` and/or `crates/autospec-cli/src/commands/graph.rs`. Preferred long-term implementation: Rust. A shell/Python prototype MAY be used first if needed.

### 27.6 Project Board Dependency Projection

Existing dependency projection MUST continue to treat only hard dependencies as readiness edges. Do NOT ingest parallel peers, conflict domains, shared ownership, or parent issue as blockers.

### 27.7 Documentation

Update `docs/concepts.md`, `docs/USER_MANUAL.md`, `docs/API_REFERENCE.md`, `docs/CONFIG_REFERENCE.md`, `SKILLS.md`.

---

## 28. Proposed Rust Data Structures

```rust
pub struct IssueGraph {
    pub issues: Vec<PlannedIssue>,
    pub hard_edges: Vec<DependencyEdge>,
}

pub struct PlannedIssue {
    pub id: String,
    pub title: String,
    pub ownership: Ownership,
    pub concurrency: ConcurrencyMetadata,
}

pub struct DependencyEdge {
    pub predecessor: String,
    pub successor: String,
    pub reason: DependencyReason,
    pub artifact: Option<String>,
}

pub enum DependencyReason {
    ConsumesNewInterface,
    ConsumesNewType,
    ConsumesNewSchema,
    ConsumesMigration,
    ConsumesGeneratedArtifact,
    RequiresStructuralMigration,
    RequiresNewProtocol,
    VerificationRequiresPredecessor,
    ExternalPrerequisite,
}

pub struct Ownership {
    pub exclusive: Vec<OwnedSurface>,
    pub shared_read: Vec<OwnedSurface>,
    pub shared_write: Vec<OwnedSurface>,
}

pub struct OwnedSurface {
    pub path: String,
    pub symbols: Vec<String>,
}

pub struct ConcurrencyMetadata {
    pub parallel_safe: bool,
    pub conflict_domains: Vec<String>,
}

pub struct GraphMetrics {
    pub issue_count: usize,
    pub hard_edge_count: usize,
    pub root_count: usize,
    pub leaf_count: usize,
    pub initial_width: usize,
    pub maximum_width: usize,
    pub critical_path_length: usize,
    pub average_wave_width: f64,
    pub shared_write_hotspots: usize,
    pub parallelization_score: u8,
}
```

---

## 29. Algorithm Requirements

### 29.1 Cycle Detection

Use topological sort. Any cycle MUST be fatal before issue creation.

### 29.2 Execution Waves

Kahn-style topological layering:

```text
wave 0 = zero-indegree nodes
remove wave 0
wave 1 = new zero-indegree nodes
...
```

### 29.3 Critical Path

```text
distance[node] = 1 + max(distance[pred])
```

Maximum distance is critical-path length.

### 29.4 Maximum Width

For v1, maximum topological wave size is sufficient. Do not attempt NP-hard optimal resource scheduling.

### 29.5 Ownership Overlap

Normalize paths. Detect: exact same file; parent directory glob vs child file; same declared symbol; same generated artifact.

Do not infer symbol overlap from text heuristics in v1.

---

## 30. Capacity-Aware Planner Heuristics

1. Create the semantically correct issue set.
2. Remove all ordering assumptions.
3. Mark concrete artifact dependencies only.
4. Partition shared write surfaces.
5. If fleet capacity is underutilized, look for meaningful additional splits: adapters; independent endpoints; independent migration tooling; compatibility layers; observability; benchmarks; documentation; independent test harness work; fixtures; UI components with stable contracts.
6. If excessive write collisions appear, merge or repartition issues.
7. Recompute graph metrics.

---

## 31. Examples

### 31.1 Bad

```text
#1 core -> #2 API -> #3 CLI -> #4 tests -> #5 docs
```

Initial width: `1`

### 31.2 Better

```text
       +-> #2 API + API tests
#1 ----+-> #3 CLI + CLI tests
       +-> #4 metrics
       +-> #5 docs
```

Initial width: `1`. After #1: `4`.

### 31.3 Best When Contract Already Exists

```text
#1 core behavior + focused tests
#2 API + focused tests
#3 CLI + focused tests
#4 metrics
#5 docs
```

Initial width: `5`. No dependency merely because all belong to the same feature.

---

## 32. TDD and Testing Rules

Parallelization MUST NOT produce separate "write implementation" and "later add unit tests" chains by default.

Each implementation issue SHOULD include focused tests.

```text
#A implement parser + parser tests
#B implement CLI adapter + CLI tests
#C implement HTTP adapter + HTTP tests
```

A later integration issue MAY depend on A/B/C if it genuinely verifies their combined behavior.

Dedicated testing issues remain appropriate for: shared test harnesses; E2E infrastructure; compatibility matrices; performance benchmarking; independent adversarial/security tests; reusable fixtures.

---

## 33. Architecture Quality Guardrail

The concurrency optimizer MUST NOT:

- duplicate architecture to avoid dependencies;
- introduce parallel implementations of the same abstraction;
- clone shared utilities into separate modules;
- bypass common interfaces;
- duplicate schemas;
- introduce unnecessary getters/setters;
- generate one-off wrappers purely to create ownership boundaries;
- create artificial service boundaries;
- create fragmented code that increases long-term complexity.

Concurrency is subordinate to coherent architecture.

---

## 34. Acceptance Criteria

- **AC-1 Capacity Input** — `autospec-define` accepts or derives an effective agent capacity between 10 and 100.
- **AC-2 Dependency Proof** — every hard dependency has a valid reason code and justification.
- **AC-3 Split Independence** — a TOO_MANY_FILES split does not automatically create a dependency.
- **AC-4 Ownership** — every generated child issue includes explicit write ownership.
- **AC-5 Concurrency Metadata** — every child issue includes a `## Concurrency` section.
- **AC-6 Graph Validation** — the DAG is checked for cycles before GitHub issue creation.
- **AC-7 Metrics** — the planner reports issue count; hard-edge count; initial width; maximum wave width; critical path; fleet saturation; parallelization score.
- **AC-8 Retry** — low-width plans trigger exactly one concurrency optimization retry.
- **AC-9 Ready Queue Compatibility** — existing dependency-aware `autospec-run` tests continue passing.
- **AC-10 Parent Independence** — parent/sub-issue relationships do not become readiness blockers.
- **AC-11 Conflict Separation** — shared-write or merge-conflict risk does not automatically create `Depends on`.
- **AC-12 Architecture** — no decomposition rule requires duplication of a shared architectural contract.

---

## 35. Test Plan

### 35.1 Unit Tests

Graph tests for: empty graph; one node; wide root graph; linear chain; diamond; multiple roots; fan-in; fan-out; cycle detection; critical path; wave generation; ownership overlap; score calculation.

### 35.2 Lint Tests

Test every `AS-DAG-*` code.

### 35.3 Skill Golden Tests

Verify all harness variants contain matching dependency rules; concurrency pass; fleet capacity rules; revised TOO_MANY_FILES guidance.

### 35.4 Ready Queue Regression

Existing tests must continue validating that only hard dependencies block issues.

Fixture `#A and #B share conflict domain` — expected: both ready.
Fixture `#B Depends on #A` — expected: `#B` blocked.

### 35.5 Capacity Tests

Run representative decompositions with `10`, `20`, `32`, `50`, `75`, `100`. Validate that larger capacity encourages greater useful decomposition without producing trivial issue spam.

### 35.6 Real-World Golden Specs

At least three fixtures: CLI/backend feature; UI/API/persistence feature; large cross-cutting architecture feature.

Record graph metrics before and after concurrency optimization. Expected: equal or lower critical path; equal or greater initial width; no lost acceptance criteria; no new architecture violations.

---

## 36. Rollout

- **Stage 1 — Advisory.** Add analyzer; metrics; lint warnings; prompt changes. No decomposition is rejected except cycles/malformed dependencies.
- **Stage 2 — Optimization Retry.** Enable one automatic concurrency-review retry.
- **Stage 3 — Required Dependency Justification.** Hard dependencies without machine-readable justification fail issue validation.
- **Stage 4 — Orchestrator Integration.** Expose metadata to fleet schedulers.

---

## 37. Migration

Existing issues remain valid. Old issues without `## Concurrency` are treated as:

```yaml
parallel_safe: unknown
ownership: unknown
```

No migration is required for existing ready queues. Newly generated issues MUST use the new format.

---

## 38. Observability

Emit planning telemetry:

```text
autospec.define.issue_count
autospec.define.hard_edge_count
autospec.define.initial_width
autospec.define.maximum_width
autospec.define.critical_path
autospec.define.parallelization_score
autospec.define.fleet_capacity
autospec.define.initial_saturation
autospec.define.shared_write_hotspots
autospec.define.edges_removed_by_optimizer
autospec.define.issues_split_by_optimizer
autospec.define.issues_merged_by_optimizer
```

---

## 39. Post-Execution Learning

After `autospec-run` completes, compare predicted concurrency with actual behavior. Capture: predicted initial width; actual concurrent workers; merge conflicts; rebase frequency; failed ownership assumptions; dependency blocks; average worker idle time; issue duration.

Out of scope for the initial implementation, but the telemetry schema SHOULD make it possible.

---

## 40. Implementation Backlog

### Wave 0 — Independent foundation work

**Issue A — Graph Model and Metrics.** Ownership: `crates/autospec-core/src/graph/**`. Implement graph model; cycle detection; wave generation; critical path; root/leaf metrics; saturation metrics; score calculation. Dependencies: none.

**Issue B — Ownership and Concurrency Schema.** Ownership: new graph/planning metadata structs/schema files. Implement ownership model; concurrency model; dependency reason enum; serialization/deserialization. Dependencies: none.

**Issue C — DAG Lint Rules.** Ownership: lint rule definitions and isolated lint tests. Implement AS-DAG-001 through AS-DAG-010. Dependencies: none if interfaces are defined locally and reconciled during integration.

**Issue D — Skill Prompt: Dependency Minimization.** Ownership: `skills/autospec-define/**`. Implement hard-dependency proof rules; fleet objective; concurrency optimizer prompt; TOO_MANY_FILES rewrite. Dependencies: none.

**Issue E — autospec-split Prompt Parity.** Ownership: `skills/autospec-split/**`. Mirror decomposition behavior. Dependencies: none.

**Issue F — Issue Skeleton Concurrency Sections.** Ownership: `scripts/gen-issue-skeleton.sh` and related skeleton tests. Add `## Concurrency` and `## Dependency justification`. Dependencies: none.

**Issue G — Configuration and Capacity Input.** Ownership: config parser / CLI planning arguments. Add `--agents`; env fallback; config fallback; default 32; clamp/validation. Dependencies: none.

**Issue H — Documentation.** Ownership: docs only. Draft documentation for new concurrency concepts and configuration. Dependencies: none.

### Wave 1 — Integration work

**Issue I — DAG Analyzer CLI.** Consume graph model from A and metadata from B. Command `autospec graph analyze`. Depends on: A, B.

**Issue J — Issue Lint Integration.** Wire C plus B into issue validation. Depends on: B, C.

**Issue K — Phase 3 Analyzer Integration.** Wire capacity, graph analysis, and planner output into `autospec-define`. Depends on: A, B, G.

**Issue L — Structured Skeleton Metadata.** Wire B into F if machine-readable issue blocks are implemented in v1. Depends on: B, F.

### Wave 2 — Optimization loop

**Issue M — Concurrency Review Retry.** Implement score threshold; one retry; before/after graph comparison; optimizer change summary. Depends on: D, K.

**Issue N — Execution Wave Reporting.** Add CLI and `autospec-define` output formatting. Depends on: I, K.

**Issue O — Ready Queue Conflict Regression Tests.** Prove conflict metadata does not block readiness. Depends on: F or L only if fixture format requires new body sections; otherwise Wave 0.

### Wave 3 — Final integration

**Issue P — Full Pipeline E2E.** Run spec -> decomposition -> concurrency optimization -> issue creation -> classification -> ready queue. Validate 10/32/50/100 capacity fixtures. Depends on: M, N, J.

**Issue Q — Skill Goldens and Release Documentation.** Regenerate all synchronized skill outputs and finish docs. Depends on: D, E, M.

---

## 41. Expected Concurrency of This Implementation

Initial wave: `A B C D E F G H` — eight agents immediately.
Wave 1: `I J K L` — four agents.
Wave 2: `M N O` — three agents.
Final: `P Q`.

This implementation does not itself saturate a 100-agent fleet because the feature is not large enough to justify 100 meaningful work packages. That is intentional.

Fleet saturation means:

> expose every useful independent engineering unit,

not:

> manufacture work until every worker is busy.

---

## 42. Definition of Done

1. `autospec-define` explicitly optimizes decomposition for 10–100 implementation agents;
2. issue-size splitting no longer implies sequencing;
3. every hard dependency is justified;
4. ownership and conflict information are distinct from dependencies;
5. deterministic graph metrics exist;
6. a low-width graph triggers one optimization retry;
7. generated issues expose concurrency metadata;
8. existing `autospec-run` readiness semantics remain compatible;
9. regression tests prove conflict-risk metadata does not block parallel execution;
10. docs explain capacity-aware decomposition;
11. representative large specs show materially improved graph width without architecture degradation.

---

## 43. Recommended Default Policy

```yaml
planning:
  parallelism:
    target_agents: 32
    min_supported_agents: 10
    max_supported_agents: 100

    target_initial_width_ratio: 0.60
    optimization_retry_threshold: 65
    max_optimization_retries: 1

    max_dependency_fan_in_before_warning: 5
    shared_write_hotspot_threshold: 3

    require_dependency_justification: true
    size_split_implies_dependency: false
    parent_relationship_implies_dependency: false
    conflict_risk_implies_dependency: false
```

---

## 44. Final Design Rule

The most important invariant is:

> **AutoSpec MUST serialize work only when correctness requires serialization.**

Everything else — issue ordering, implementation convenience, merge-risk management, project organization, documentation order, and parent-child grouping — must remain separate from the hard dependency graph.

For a 10–100 agent fleet, the issue DAG is a scheduling artifact. Its width is a first-class quality property.
