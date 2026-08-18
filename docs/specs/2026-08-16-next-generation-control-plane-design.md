# AutoSpec Next-Generation Control Plane and Learning Architecture

**Status:** Proposed implementation specification  
**Date:** 2026-08-16  
**Project:** AutoSpec  
**Primary repository:** `berlinguyinca/autospec`  
**Target implementation language:** Rust-first for durable core primitives, with additive compatibility for existing shell/skill workflows  
**Purpose:** Close the remaining major conceptual gaps in AutoSpec and establish a bounded path from repository-level autonomous execution to a self-improving, outcome-aware engineering control plane.

**Related:** the seven other `docs/specs/2026-08-16-*` designs and
`docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md` (decisions D1–D9).

## Workstream ownership map

This is the eighth specification landed on 2026-08-16. Three of its eight workstreams
are genuinely new; five restate subsystems that already have an owner. Where this
document and the owner disagree, **the owner wins** and this spec consumes it.

**Genuinely new — the reason to implement this spec:**

| Workstream | Why it is new |
|---|---|
| **A — Context Compiler** (§6) | Nothing else specifies *what to put in an agent's prompt*. Note the name collision: the multi-model spec's §11 "context as a schedulable resource" is GPU KV-cache budget, an entirely different concern. Do not conflate them. |
| **B — Complexity Governor / dynamic replanning** (§7) | Detecting that implementation reality has invalidated the plan. Adjacent to change-graph §47 replanning, which is the *mechanism*; this is the *trigger*. |
| **H — Engineering Policy Compiler** (§13) | Promoting repeated human/reviewer guidance into durable policy. AS-AEO-001's policy engine *enforces* policy; nothing else *derives* it. |

**Delegating — do not reimplement:**

| Workstream | Owner |
|---|---|
| **C — Post-merge outcome learning** (§8) | Multi-model §24–§29 (learned utility routing) and the RealWork spec (**D6**). Predictions and actuals append to the existing ledger — **D3**: one ledger, never fork. |
| **D — Requirements-to-outcome traceability** (§9) | AS-AEO-001 §47 and Epic 7 (evidence and assurance), which already wrap the shipped `core::evidence` layer. |
| **E — Tournament mode** (§10) | Independence and separation-of-duties rules are fixed by multi-model §4 and **D2**'s 14-role vocabulary. This spec supplies candidate *generation*; it does not redefine who may review whom. |
| **F — Repository auto-onboarding and calibration** (§11) | Calibration is the RealWork spec's (**D6**); capability evidence levels are multi-model §8. This spec supplies repository *discovery*, not a second calibration engine. |
| **G — Organization / multi-repo control plane** (§12) | The existing fleet layer owns node and repository placement; change-graph §48–§49 owns cross-repository change. **D8** applies unchanged. |

## Consistency with recorded decisions

- **§14 shared data model** — the eighteen record types are additive and welcome, but per
  **D5** and **D7** they persist in the **one** shared database, not a new store. Explicit
  versioning and migration rules (§14) must use the same shared-migration protocol the
  resource ledger establishes, or two subsystems will collide on one `user_version`.
- **§15 provenance contract** — compatible with, and should be unified with, the routing
  ledger's explainability output rather than emitted as a parallel decision log. **D3**
  and **D4** both point the same way: one explainable decision record, not two.
- **§16 benchmark integration** — routes through the RealWork spec per **D6**.

## Decomposition gate

Workstream A is the only one with no unbuilt dependency and could be decomposed once
the executor chain lands. B, C, E and F all consume the router, the ledger extensions,
or the benchmark corpus — none of which exist yet. G consumes the change graph, itself
gated. H consumes AS-AEO-001's policy engine (Epics 1–2).

**Nothing here is decomposable before the executor chain (#3172/#3173) and
resource-lifecycle Phase 1 merge.** This spec's own §1 says the same thing in different
words: *"After these capabilities are implemented and validated … further work should
emphasize implementation quality, dogfooding, benchmarks, reliability and user
experience rather than continued feature invention."* That guidance applies to this
document too — it is the eighth design landed in one day against one merged
implementation PR.

---

## 1. Executive Summary

AutoSpec already covers most of the repository-level autonomous software-development lifecycle:

- intent capture;
- specification;
- issue decomposition;
- model-fit classification;
- autonomous task selection;
- worktree/runtime isolation;
- implementation;
- testing and validation;
- independent review;
- security, QA, UI/UX, accessibility and documentation gates;
- PR creation and merge orchestration;
- rollback and provenance;
- local/cloud model routing;
- cost/usage governance;
- repository story and memory;
- continuous discovery and never-idle operation.

The remaining architectural work is not primarily “more agents” or “more scanners.” The largest remaining opportunities are higher-order control-plane capabilities that let AutoSpec reason about *what context to provide, when to replan, what outcome was achieved, what was learned, and how work spans repositories*.

This specification defines eight major workstreams:

1. **Context Compiler**
2. **Complexity Governor and Dynamic Replanning**
3. **Post-Merge Outcome Learning**
4. **Requirements-to-Outcome Traceability**
5. **Competitive Solution / Tournament Mode**
6. **Repository Auto-Onboarding and Calibration**
7. **Organization / Multi-Repo Control Plane**
8. **Engineering Policy Compiler**

The intended end state is an AutoSpec architecture with six clear layers:

1. Organization Control
2. Intent / Product
3. Planning
4. Intelligence
5. Execution
6. Evidence / Learning

After these capabilities are implemented and validated, AutoSpec should be considered conceptually complete at the major-system level. Further work should emphasize implementation quality, dogfooding, benchmarks, reliability and user experience rather than continued feature invention.

---

# 2. Goals

AutoSpec MUST evolve from an autonomous coding workflow into an evidence-driven engineering control plane that:

- gives each agent the minimum sufficient, purpose-specific context;
- detects when implementation reality invalidates the original plan;
- learns from estimated-vs-actual execution and post-merge outcomes;
- preserves lineage from human intent through production evidence;
- can use multiple independent candidate solutions for high-value work;
- can onboard unfamiliar repositories with minimal manual setup;
- can coordinate changes across multiple repositories;
- converts repeated human/reviewer guidance into durable engineering policy;
- preserves separation of duties;
- remains interruptible, inspectable and cost-bounded;
- remains usable in single-repository mode without requiring the multi-repo layer.

---

# 3. Non-Goals

This work MUST NOT:

- replace the existing validated AutoSpec workflows in one rewrite;
- remove the current shell/skill execution path before Rust equivalents are proven;
- introduce a hosted SaaS requirement;
- require a proprietary model provider;
- allow models to silently rewrite their own evaluation criteria;
- permit implementers to modify immutable verification evidence to make themselves pass;
- introduce cross-repository writes without explicit repository policy;
- optimize for token reduction at the expense of task correctness;
- turn every task into tournament mode;
- manufacture work merely to keep agents busy;
- make AutoSpec dependent on one organization structure or monorepo layout.

---

# 4. Architectural Principles

## 4.1 Evidence before confidence

AutoSpec MUST prefer measurable evidence over model assertions.

Examples:

- tests over “looks correct”;
- measured latency over “should be faster”;
- accepted user metric over “better UX”;
- historical outcome data over model reputation;
- context coverage evidence over prompt size alone.

## 4.2 Minimal sufficient context

Agents SHOULD receive the smallest context package that preserves success probability.

More context is not automatically better.

## 4.3 Replanning is normal

A plan is a hypothesis. AutoSpec MUST treat unexpected implementation complexity as a first-class event, not as an exception to hide.

## 4.4 Separation of powers

Planner, implementer, verifier and approver roles MUST remain distinguishable and auditable.

The same model MAY occupy multiple roles only when allowed by policy, but the same execution instance MUST NOT self-approve its own implementation evidence.

## 4.5 Learning must be reversible

AutoSpec MUST distinguish:

- raw observations;
- derived metrics;
- learned heuristics;
- promoted policies.

Learned behavior MUST be versioned and revertible.

## 4.6 Single-repo remains first-class

All organization-level features MUST degrade cleanly to one repository.

---

# 5. Target Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                ORGANIZATION CONTROL                     │
│ repo graph · portfolio · policies · cross-repo plans   │
├─────────────────────────────────────────────────────────┤
│                   INTENT / PRODUCT                      │
│ goals · requirements · outcomes · priorities           │
├─────────────────────────────────────────────────────────┤
│                      PLANNING                           │
│ specs · issue DAG · estimates · complexity governor    │
├─────────────────────────────────────────────────────────┤
│                    INTELLIGENCE                         │
│ context compiler · routing · benchmarks · tournaments  │
│ calibration · learned heuristics                       │
├─────────────────────────────────────────────────────────┤
│                     EXECUTION                           │
│ agents · worktrees · stacked PRs · tests · review      │
│ QA · UI · docs · security · release                    │
├─────────────────────────────────────────────────────────┤
│                 EVIDENCE / LEARNING                     │
│ provenance · outcomes · telemetry · traceability       │
│ model performance · policy learning                    │
└─────────────────────────────────────────────────────────┘
```

---

# 6. Workstream A — Context Compiler

## 6.1 Problem

AutoSpec currently makes increasingly sophisticated choices about which model should perform a task, but model performance also depends heavily on *what the model receives*.

Passing a whole repository or large undifferentiated prompt:

- wastes tokens;
- reduces useful signal;
- increases search wandering;
- hurts smaller/local models disproportionately;
- makes review less independent;
- makes benchmark results less comparable.

## 6.2 Goal

Create a deterministic, inspectable context-building subsystem that produces role-specific context bundles.

## 6.3 Required CLI

```bash
autospec context build <issue-or-task>
autospec context build <issue-or-task> --role planner
autospec context build <issue-or-task> --role implementer
autospec context build <issue-or-task> --role verifier
autospec context explain <issue-or-task>
autospec context audit <issue-or-task>
autospec context compare <bundle-a> <bundle-b>
```

## 6.4 Context sources

The compiler SHOULD consider:

- issue body;
- parent/child issue lineage;
- current spec;
- acceptance criteria;
- related ADRs;
- referenced code paths;
- dependency graph;
- call graph where available;
- tests covering touched code;
- recent relevant PRs;
- git blame/history;
- public APIs;
- database/schema dependencies;
- configuration;
- current failures;
- prior failed attempts;
- repository policy;
- model-specific context limits.

## 6.5 Role-specific bundles

### Planner bundle

SHOULD emphasize:

- architecture;
- interfaces;
- dependencies;
- historical changes;
- scope boundaries;
- constraints;
- risks.

### Implementer bundle

SHOULD emphasize:

- exact files and symbols;
- tests;
- acceptance criteria;
- coding conventions;
- relevant interfaces;
- editable/non-editable boundaries.

### Verifier bundle

MUST preserve independence.

It SHOULD include:

- requirements;
- acceptance criteria;
- diff;
- tests;
- verification commands;
- architecture/security policy;

and SHOULD NOT blindly inherit the implementer’s full reasoning transcript.

### Documentation/UI/UX roles

SHOULD receive surface-specific context only where capability detection justifies it.

## 6.6 Bundle manifest

Each bundle MUST include a manifest similar to:

```yaml
bundle_version: 1
task_id: issue-123
role: implementer
generated_at: ...
repo_commit: ...
token_estimate: 18422
sources:
  - path: src/foo.rs
    reason: direct_symbol_dependency
    confidence: 0.98
  - path: tests/foo_test.rs
    reason: behavioral_coverage
    confidence: 0.95
excluded:
  - path: docs/legacy.md
    reason: stale
```

## 6.7 Context quality metrics

AutoSpec MUST benchmark:

- task success;
- tokens consumed;
- wall-clock time;
- retry count;
- cost;
- missing-context failures;
- irrelevant-context ratio where measurable.

Primary derived metrics:

- **success per 1K tokens**
- **success per dollar**
- **success per minute**
- **context efficiency**
- **context recall for required evidence**

## 6.8 Acceptance criteria

- Two agents handling the same benchmark task can receive reproducibly generated bundles.
- Bundle generation is deterministic for the same repo state and config.
- `context explain` shows why every included source was selected.
- `context audit` detects missing acceptance criteria, missing test context and stale source inclusion.
- Routing can prefer a smaller/local model when the compiled context makes the task eligible.
- Benchmark reports include context-strategy results, not only model results.

---

# 7. Workstream B — Complexity Governor and Dynamic Replanning

## 7.1 Problem

Issue decomposition is performed before implementation, but implementation frequently reveals hidden coupling, migrations, broader blast radius or invalid assumptions.

Agents should not continue indefinitely merely because a task was originally classified as small.

## 7.2 Goal

Detect material divergence between planned complexity and observed complexity, suspend safely, and return the task to planning.

## 7.3 Signals

The governor SHOULD track:

- expected vs actual files touched;
- expected vs actual lines changed;
- modules/packages crossed;
- dependency additions;
- database/schema changes;
- public API changes;
- number of failed implementation attempts;
- test expansion;
- compile/test duration growth;
- blast-radius score;
- token/context growth;
- agent self-reported uncertainty as a weak signal only.

## 7.4 State transition

```text
READY
  ↓
IMPLEMENTING
  ↓
COMPLEXITY_THRESHOLD_EXCEEDED
  ↓
SUSPENDED_FOR_REPLAN
  ↓
REPLANNED
  ↓
NEW_ISSUE_DAG
  ↓
RESUME
```

## 7.5 Required behavior

When a configured threshold is exceeded, AutoSpec MUST:

1. stop editing at a safe checkpoint;
2. preserve the worktree and evidence;
3. produce a divergence report;
4. return the task to a planner;
5. update estimates;
6. split/rewrite the issue DAG if necessary;
7. preserve useful partial work when safe;
8. resume through the normal queue.

It MUST NOT silently widen the original issue.

## 7.6 Example divergence report

```yaml
planned:
  files: 5
  modules: 1
  db_change: false
  public_api_change: false

observed:
  files: 21
  modules: 4
  db_change: true
  public_api_change: true

trigger:
  - file_count_ratio > 3
  - fenced_surface_detected
  - schema_change_discovered
```

## 7.7 Stacked PR integration

Dynamic replanning SHOULD prefer a stacked PR decomposition when:

- later changes depend on a foundational refactor;
- a migration can be separated from behavior;
- test infrastructure must land first;
- multiple independently reviewable layers exist.

## 7.8 Acceptance criteria

- A task can be suspended without losing evidence.
- Hidden schema/API changes automatically force replanning or quarantine according to policy.
- Replanning preserves issue lineage.
- A large unexpected task cannot silently merge under the original small-task risk classification.
- Stacked PRs can be generated from a replanned DAG.

---

# 8. Workstream C — Post-Merge Outcome Learning

## 8.1 Problem

Passing tests and merging a PR proves implementation quality at merge time, but does not prove the original prediction was accurate or the product outcome was achieved.

## 8.2 Goal

Create a learning loop based on predicted-vs-actual execution and post-merge outcomes.

## 8.3 Predictions to record

Before implementation AutoSpec SHOULD record:

- expected task duration;
- expected tokens;
- expected cost;
- expected files/modules;
- expected risk;
- expected retries;
- expected performance change;
- expected user/product outcome when available;
- expected model suitability.

## 8.4 Actuals to record

After execution/observation:

- actual duration;
- actual token use;
- actual cost;
- retries;
- review rejections;
- rollback incidence;
- files/modules changed;
- production/benchmark outcome;
- defect/regression reports;
- follow-up issue count.

## 8.5 Learning targets

AutoSpec MAY update:

- model suitability by task class;
- effort estimation;
- decomposition heuristics;
- risk scoring;
- context selection;
- reviewer routing;
- test strategy;
- tournament eligibility;
- cost prediction.

## 8.6 Learning safeguards

Raw history MUST remain immutable.

Learned coefficients and recommendations MUST be:

- versioned;
- attributable to evidence;
- bounded;
- revertible;
- promoted only after validation.

A single unusual task MUST NOT massively reweight routing.

## 8.7 CLI

```bash
autospec learn observe <work-item>
autospec learn report
autospec learn model-performance
autospec learn estimator
autospec learn rollback <learning-version>
```

## 8.8 Acceptance criteria

- Every completed task can produce predicted-vs-actual metrics.
- Model routing can use repository-specific historical performance.
- Estimation error is measurable over time.
- Learned routing changes carry provenance.
- Learning can be disabled without disabling normal execution.

---

# 9. Workstream D — Requirements-to-Outcome Traceability

## 9.1 Goal

Create a durable lineage graph from original intent to implementation and observed outcome.

## 9.2 Core lineage

```text
Intent
 ↓
Goal
 ↓
Requirement
 ↓
Spec
 ↓
Acceptance Criterion
 ↓
Issue
 ↓
PR
 ↓
Code
 ↓
Test
 ↓
Release/Deployment
 ↓
Outcome Signal
```

## 9.3 Required questions AutoSpec must answer

AutoSpec SHOULD be able to answer:

- Why does this code exist?
- Which requirement does this test protect?
- Which requirements lack test evidence?
- Which acceptance criteria have no implementation evidence?
- Which code paths have no current requirement lineage?
- Which requirements were superseded?
- Which production metric was expected to change?
- Did that metric improve?
- Which PR introduced this behavior?

## 9.4 Storage

A repository-local graph SHOULD live under a durable schema such as:

```text
.autospec/trace/
  nodes.jsonl
  edges.jsonl
  snapshots/
```

Rust core SHOULD expose typed node and edge structures.

## 9.5 Node examples

- `intent`
- `goal`
- `requirement`
- `acceptance_criterion`
- `issue`
- `pull_request`
- `commit`
- `symbol`
- `test`
- `deployment`
- `metric`
- `incident`

## 9.6 Acceptance criteria

- Trace links survive issue/PR closure.
- Generated reports cite evidence.
- Orphan requirements and orphan code can be detected.
- AutoSpec can render a trace report for an issue, PR, file or requirement.
- Trace graph updates are additive and auditable.

---

# 10. Workstream E — Competitive Solution / Tournament Mode

## 10.1 Problem

Independent review helps catch errors, but for sufficiently high-value work it can be beneficial to generate multiple solutions rather than one solution plus one reviewer.

## 10.2 Goal

Allow AutoSpec to run multiple isolated solution lanes and select the strongest candidate using objective evidence.

## 10.3 Eligibility

Tournament mode SHOULD be restricted to tasks where expected benefit justifies cost.

Candidate triggers:

- high severity;
- architectural decision;
- performance optimization;
- security-sensitive change;
- historically difficult task class;
- multiple plausible approaches;
- high-value bug;
- benchmark/research tasks.

## 10.4 Example

```text
                 Task
                   │
       ┌───────────┼───────────┐
       ▼           ▼           ▼
    Candidate A Candidate B Candidate C
       │           │           │
       └───────────┼───────────┘
                   ▼
              Evaluator
                   │
     ┌─────────────┼─────────────┐
     ▼             ▼             ▼
 correctness   performance   maintainability
 security      complexity    cost
                   │
                   ▼
                 Winner
```

Candidates MAY come from:

- different models;
- the same model with isolated contexts/seeds/strategies;
- local vs cloud model comparisons.

## 10.5 Fairness

All candidates MUST receive:

- equivalent requirements;
- equivalent acceptance criteria;
- equivalent immutable verification criteria.

The evaluator MUST NOT reward a candidate merely for using the same model family.

## 10.6 Evaluation

Candidate scoring SHOULD combine:

- required tests;
- hidden/immutable tests where applicable;
- performance;
- security;
- diff size;
- complexity;
- maintainability;
- policy compliance;
- cost;
- execution time.

## 10.7 Acceptance criteria

- Tournament lanes are fully isolated.
- Candidate evaluations are reproducible.
- Losing candidates are preserved as evidence until retention cleanup.
- Tournament mode is never the default for cheap routine tasks.
- Routing learns whether tournaments were worth their additional cost.

---

# 11. Workstream F — Repository Auto-Onboarding and Calibration

## 11.1 Goal

Allow AutoSpec to inspect an unfamiliar repository and create a trustworthy operating profile with minimal manual setup.

## 11.2 Proposed CLI

```bash
autospec init --learn
autospec onboard scan
autospec onboard report
autospec onboard calibrate
autospec onboard apply
```

## 11.3 Discovery targets

AutoSpec SHOULD discover:

- languages;
- frameworks;
- build tools;
- test frameworks;
- lint/format commands;
- CI workflows;
- repository layout;
- packages/services;
- dependency graph;
- public API surfaces;
- migrations;
- generated files;
- dangerous/fenced surfaces;
- docs;
- ownership patterns;
- recent PR history;
- release process;
- deployment tooling;
- available UI/browser surfaces;
- security tooling.

## 11.4 Historical calibration

Where repository history is available, AutoSpec SHOULD sample merged work to generate replay tasks for model calibration.

Historical work MAY be used to evaluate:

- planning;
- implementation;
- code review;
- bug diagnosis;
- test generation;
- documentation;
- UI/vision tasks.

## 11.5 Output

Example:

```text
Repository confidence: 0.91

Detected:
✓ Rust workspace
✓ GitHub Actions
✓ cargo test
✓ cargo clippy
✓ release tags
✓ migration surface
✓ CLI surface
✓ docs corpus

Safe autonomy recommendation: Level 3

Human/fenced approval recommended for:
- schema migrations
- auth
- release credentials
- workflow permission changes

Historical calibration set:
47 eligible work items

Recommended role routing:
planner: Claude-class
implementer-small: local Qwen-class
implementer-large: Codex-class
reviewer: independent higher-class model
```

## 11.6 Apply gate

`autospec onboard apply` MUST require an explicit policy-approved transition before writing durable repository configuration.

Scanning and reporting SHOULD be non-mutating by default.

## 11.7 Acceptance criteria

- An unfamiliar supported repo can produce a usable AutoSpec profile automatically.
- AutoSpec clearly distinguishes detected facts from inferred recommendations.
- Model calibration uses real repository work where available.
- Generated config is reviewable before activation.
- Unsupported capabilities fail safely.

---

# 12. Workstream G — Organization / Multi-Repo Control Plane

## 12.1 Goal

Coordinate work that spans multiple repositories while preserving repository autonomy and safety boundaries.

## 12.2 Organization graph

AutoSpec SHOULD model:

- repositories;
- packages;
- services;
- APIs;
- deployment dependencies;
- shared libraries;
- consumers/providers;
- compatibility constraints;
- owners;
- release relationships.

## 12.3 Example

```text
Organization
 ├── auth-lib
 │      ↓
 ├── api
 │   ├── frontend
 │   └── mobile
 │
 └── deployment
```

A change such as “upgrade authentication flow” may become:

```text
EPIC
 ├── auth-lib #81
 ├── api #419
 ├── frontend #872
 ├── mobile #302
 └── deployment #117
```

## 12.4 Cross-repo plan

A cross-repo plan MUST include:

- dependency ordering;
- compatibility strategy;
- integration gates;
- release sequencing;
- rollback strategy;
- owner/policy boundaries;
- repository-specific models and context.

## 12.5 Cross-repo writes

Cross-repo mutation MUST be policy-controlled.

Default mode SHOULD be:

```text
discover → propose → simulate → approve/policy-gate → execute
```

Repositories MAY independently declare:

- read-only;
- proposal-only;
- issue-write;
- branch/PR-write;
- merge-eligible.

## 12.6 Compatibility testing

AutoSpec SHOULD support:

- consumer/provider contract tests;
- version compatibility matrices;
- staged rollout;
- coordinated stacked PRs across repositories;
- ephemeral integration environments where available.

## 12.7 Acceptance criteria

- Multi-repo discovery works without write access.
- Cross-repo dependency order is explicit.
- One repository can block only the dependent portion of a plan.
- Rollback plans preserve cross-repo compatibility.
- Single-repo execution remains unaffected when organization mode is disabled.

---

# 13. Workstream H — Engineering Policy Compiler

## 13.1 Problem

Humans repeatedly communicate durable engineering preferences through:

- review comments;
- rejected PRs;
- ADRs;
- contribution guides;
- coding standards;
- recurring fixes.

Those preferences are often forgotten by future agents.

## 13.2 Goal

Infer recurring engineering guidance and propose machine-enforceable policy without allowing unreviewed model opinions to become rules.

## 13.3 Inputs

Policy inference MAY inspect:

- accepted/rejected review comments;
- recurring requested changes;
- ADRs;
- CONTRIBUTING;
- architecture docs;
- linter configuration;
- code patterns;
- repeated revert causes;
- operator-authored AutoSpec policy.

## 13.4 Policy classes

### Deterministic rules

Examples:

- forbidden dependency;
- no edits to generated files;
- architectural layer boundaries;
- required integration tests for API changes;
- migration constraints.

### Review policies

Examples:

- maintainability expectations;
- UX conventions;
- naming conventions not suitable for static lint.

### Context policies

Examples:

- always include ADR-X for authentication changes;
- include migration guide for schema tasks.

### Routing policies

Examples:

- security changes require specialist reviewer;
- UI work requires vision/automation verification.

## 13.5 Promotion pipeline

```text
Observation
   ↓
Repeated Pattern
   ↓
Policy Candidate
   ↓
Evidence Report
   ↓
Validation
   ↓
Human/policy approval where required
   ↓
Active Policy
```

AutoSpec MUST NOT silently promote a learned preference to an enforceable blocking rule.

## 13.6 Storage

```text
.autospec/policies/
  architecture.yml
  testing.yml
  security.yml
  ui.yml
  routing.yml
  context.yml
```

Each rule SHOULD contain:

- source evidence;
- confidence;
- scope;
- enforcement mode;
- introduced date/version;
- rollback reference.

## 13.7 Acceptance criteria

- Repeated review feedback can produce a policy candidate.
- Candidates show supporting evidence.
- Policies can be advisory, warning or blocking.
- Active policies are versioned and reversible.
- Deterministic policies can participate in CI/AutoSpec gates.

---

# 14. Shared Data Model

The Rust core SHOULD introduce versioned typed records for:

```rust
TaskContextBundle
ContextSource
ComplexityEstimate
ComplexityObservation
ReplanEvent
OutcomePrediction
OutcomeObservation
LearningSnapshot
TraceNode
TraceEdge
TournamentRun
TournamentCandidate
RepositoryProfile
RepositoryCapability
OrganizationGraph
CrossRepoPlan
PolicyCandidate
EngineeringPolicy
```

All serialized formats MUST be explicitly versioned.

Schema changes MUST include migration or backwards-compatible parsing rules.

---

# 15. Common Provenance Contract

Every generated decision MUST be explainable.

At minimum AutoSpec SHOULD record:

```yaml
decision:
  type: model_route
  selected: local-qwen
  alternatives:
    - codex
    - claude
  evidence:
    - historical_success_rate: 0.91
    - task_class: rust-small-change
    - context_tokens: 12000
    - predicted_cost_delta: -0.84
  policy_version: ...
  learning_version: ...
  timestamp: ...
```

This pattern SHOULD apply to:

- model routing;
- context selection;
- complexity escalation;
- tournament eligibility;
- repository autonomy level;
- policy activation;
- cross-repo execution order.

---

# 16. Benchmark Integration

These capabilities MUST integrate with the AutoSpec benchmark suite.

Benchmarks SHOULD contain real work patterns from actual repositories.

For each evaluation, record:

- task type;
- role;
- model;
- model effort level;
- context strategy;
- prompt tokens;
- completion tokens;
- tokens/sec;
- wall-clock time;
- cost;
- pass/fail;
- retries;
- reviewer score;
- immutable test score;
- context misses;
- policy violations;
- outcome where observable.

## 16.1 Context benchmark matrix

Example dimensions:

```text
model × context_strategy × task_class × effort
```

## 16.2 Tournament benchmark

Track whether:

```text
quality_gain > incremental_compute_cost
```

over time.

## 16.3 Learning benchmark

Measure whether routing/estimation improves using held-out historical tasks.

AutoSpec MUST avoid evaluating a learned rule only on the same tasks used to derive it.

---

# 17. Security and Safety Requirements

The new control-plane features MUST preserve current AutoSpec safety guarantees.

Additional requirements:

- context bundles MUST redact known secrets where possible;
- cross-repo access MUST respect repository permissions;
- policy inference MUST not ingest credentials into policy files;
- learning data MUST not include secret values;
- immutable verification artifacts MUST remain protected from implementer lanes;
- tournament candidates MUST use isolated worktrees/runtime namespaces;
- dynamic replanning MUST preserve original evidence;
- organization graphs MUST distinguish observed dependencies from inferred dependencies;
- automated policy promotion MUST be disabled by default for blocking policies.

---

# 18. Observability

A unified `autospec explain` surface SHOULD eventually support:

```bash
autospec explain route <task>
autospec explain context <task>
autospec explain estimate <task>
autospec explain replan <task>
autospec explain policy <rule>
autospec explain trace <artifact>
autospec explain tournament <run>
```

The operator should be able to answer:

- What did AutoSpec decide?
- Why?
- What evidence did it use?
- Which policy/model/learning version influenced the decision?
- Can I reproduce the decision?

---

# 19. Proposed Implementation Sequence

The implementation SHOULD occur in this order.

## Phase 1 — Context Compiler

Highest expected impact on every model and every downstream workflow.

Deliver:

- typed bundle schema;
- builder;
- role profiles;
- explain/audit;
- benchmark integration.

## Phase 2 — Complexity Governor

Prevents poor decompositions from turning into uncontrolled large changes.

Deliver:

- planned/actual telemetry;
- threshold engine;
- suspend/replan;
- stacked-PR handoff.

## Phase 3 — Outcome Learning

Start collecting data early so future phases gain history.

Deliver:

- prediction/actual records;
- repository-local learning report;
- routing/estimation inputs.

## Phase 4 — Traceability

Connect intent to implementation and outcomes.

Deliver:

- trace graph;
- orphan detection;
- trace reports.

## Phase 5 — Tournament Mode

Use context/routing/learning infrastructure already built.

Deliver:

- candidate isolation;
- objective evaluator;
- eligibility policy;
- cost-benefit reporting.

## Phase 6 — Auto-Onboarding

Use all prior primitives to bootstrap new repositories.

Deliver:

- capability scan;
- historical replay extraction;
- autonomy recommendation;
- generated config proposal.

## Phase 7 — Multi-Repo Control Plane

Only after single-repo behavior is dependable.

Deliver:

- organization graph;
- cross-repo plans;
- permissions/policy;
- compatibility orchestration.

## Phase 8 — Policy Compiler

Leverage accumulated review/outcome evidence to propose durable rules.

Deliver:

- policy candidate inference;
- evidence reports;
- advisory/warn/block modes;
- policy CI integration.

---

# 20. Suggested Epic / Issue Decomposition

## EPIC A — Context Compiler

1. Define context bundle schema.
2. Build repository symbol/dependency source resolver.
3. Add role-specific context profiles.
4. Add context explain.
5. Add context audit.
6. Add context benchmark dimensions.
7. Integrate bundle generation into dispatch.
8. Add local-model context optimization.

## EPIC B — Complexity Governor

1. Define planned-complexity schema.
2. Add runtime complexity observations.
3. Implement thresholds/policy.
4. Add suspend checkpoint.
5. Add divergence report.
6. Integrate replan workflow.
7. Add stacked-PR conversion.
8. Add complexity benchmark.

## EPIC C — Outcome Learning

1. Define prediction record.
2. Define outcome record.
3. Collect actual usage/cost/time.
4. Add review/retry outcome capture.
5. Add model-performance report.
6. Add estimator error report.
7. Integrate learned routing.
8. Add learning rollback/versioning.

## EPIC D — Traceability

1. Define trace graph schema.
2. Link intent/spec/acceptance criteria.
3. Link issues/PRs/commits.
4. Link code symbols/tests.
5. Link releases/deployments.
6. Link outcome metrics.
7. Add orphan detection.
8. Add trace report CLI.

## EPIC E — Tournament Mode

1. Define tournament schema.
2. Add eligibility scorer.
3. Add isolated candidate runners.
4. Add candidate evaluator.
5. Add winner selection.
6. Add retention/cleanup.
7. Add tournament benchmark.
8. Feed tournament outcomes into learning.

## EPIC F — Auto-Onboarding

1. Add repository capability scan.
2. Add build/test discovery.
3. Add CI discovery.
4. Add risk/fenced-surface discovery.
5. Add history sampler.
6. Add historical benchmark generation.
7. Add autonomy recommendation.
8. Add generated-config review/apply workflow.

## EPIC G — Multi-Repo

1. Define organization graph.
2. Add repo dependency discovery.
3. Add organization config.
4. Add cross-repo plan schema.
5. Add compatibility gates.
6. Add repo autonomy permission levels.
7. Add cross-repo stacked PR orchestration.
8. Add coordinated rollback.

## EPIC H — Policy Compiler

1. Define policy schema.
2. Add review-feedback extractor.
3. Add ADR/doc rule extractor.
4. Add recurring-pattern detector.
5. Add policy candidate report.
6. Add advisory/warn/block modes.
7. Add policy explain/rollback.
8. Add deterministic gate integration.

---

# 21. Definition of Done

This program is complete when AutoSpec can demonstrate all of the following on representative repositories:

1. A task receives a reproducible, role-specific context bundle.
2. Context strategy is benchmarked alongside model performance.
3. A task that unexpectedly expands is suspended and replanned automatically.
4. Predicted effort/cost/risk can be compared with actual outcomes.
5. Model routing improves based on repository-specific historical performance.
6. A requirement can be traced to issue, PR, code, test and outcome evidence.
7. High-value work can run through isolated multi-candidate tournament mode.
8. A new repository can be scanned and calibrated with minimal manual setup.
9. A multi-repo change can be planned with explicit dependency and compatibility ordering.
10. Repeated engineering feedback can become an evidence-backed policy candidate.
11. Every learned or automated decision can be explained and audited.
12. The existing single-repository AutoSpec workflow remains supported throughout the migration.

---

# 22. Conceptual Completion Criterion

Once this specification is implemented, AutoSpec SHOULD adopt a feature-governance rule:

> New major subsystems require demonstrated evidence that an unsolved engineering-control problem cannot be handled by the existing architecture.

The default product priority should become:

1. reliability;
2. benchmark quality;
3. correctness;
4. real-world dogfooding;
5. context efficiency;
6. cost efficiency;
7. UX/operator clarity;
8. ecosystem adoption;
9. performance;
10. incremental capability refinement.

This prevents AutoSpec from becoming an endlessly growing collection of agent roles and scanners.

The major conceptual architecture should at that point be treated as substantially closed.

---

# 23. Final Target State

The desired AutoSpec loop is:

```text
Human / Organization Intent
          ↓
    Requirements Graph
          ↓
       Planning
          ↓
    Context Compiler
          ↓
  Model / Effort Routing
          ↓
   Implementation Lane
          ↓
 Complexity Governor ───────┐
          ↓                 │
   Independent Verification │
          ↓                 │
   PR / Stack / Merge       │
          ↓                 │
 Post-Merge Observation     │
          ↓                 │
    Outcome Learning        │
          ↓                 │
 Traceability + Policies    │
          ↓                 │
 Better Planning / Routing ◀┘
```

Across repositories:

```text
                Organization Intent
                        ↓
                Organization Graph
                        ↓
              Cross-Repo Work Plan
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
        Repo A         Repo B        Repo C
          │             │             │
          └─────────────┼─────────────┘
                        ↓
               Compatibility Gates
                        ↓
                  Coordinated Ship
                        ↓
                  Outcome Learning
```

At this point AutoSpec is no longer merely an autonomous coding-agent workflow.

It becomes a self-improving, evidence-driven software engineering control plane.
