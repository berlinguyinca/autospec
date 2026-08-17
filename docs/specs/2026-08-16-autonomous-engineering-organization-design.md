# AutoSpec Autonomous Engineering Organization — design

## Foundational Implementation Specification

**Date:** 2026-08-16
**Specification ID:** AS-AEO-001  
**Version:** 1.0  
**Status:** Implementation-ready — Phase 0 complete; see [ADR 0001](../decisions/0001-as-aeo-001-phase-0-integration-strategy.md)  
**Priority:** P0 — Foundational  
**Implementation language:** Rust  
**Target repository:** AutoSpec  
**Compatibility requirement:** Existing AutoSpec workflows must continue to operate during migration  
**Primary objective:** Evolve AutoSpec from an AI-assisted development tool into a governed, adaptive, autonomous software-engineering organization

## Relationship to the 2026-08-16 multi-model engineering team specs

This specification **subsumes the concerns** of three specs that landed on `main`
the same day, and overlaps heavily with issues already filed from the first of them:

| Related spec | Relationship |
|---|---|
| `2026-08-16-multi-model-engineering-team-design.md` | Subsumed. Its roles/independence (§3–§4), capability advertisement (§7–§9), context scheduling (§11–§12), quota/health (§14–§15), router (§29, §43–§45), executor (§16–§17) and telemetry (§24–§28) reappear here as §16–§19, §21–§22, §32, §26, §27–§29, §67 and §58. Issues #3163–#3176 were filed from it. |
| `2026-08-16-benchmark-per-evaluation-telemetry-design.md` | Subsumed by §23 (benchmark system) and Epic 4. Not yet decomposed. |
| `2026-08-16-vision-image-generation-qualification-design.md` | Subsumed by §43–§44 (UX/vision, image generation) and Epic 9. Not yet decomposed. |

Two conflicts make this document **not directly decomposable** until §73 Phase 0
completes:

1. **Language.** This spec mandates Rust (§64 persistence, §65 core types,
   §66 module architecture). The already-filed issues #3163–#3175 implement the
   same behaviors in bash under `scripts/`, because the routing layer lives
   there today. The same subsystem must not be built twice in two languages.
2. **Premature duplication.** §73 Phase 0's own exit condition is *"approved
   migration map; no duplicate subsystem implementation started prematurely"*,
   and §72.2 requires compatibility adapters wrapping existing paths **before**
   behavior is replaced. Decomposing this spec before that map exists would
   violate its own gate.

**Resolved by [ADR 0001](../decisions/0001-as-aeo-001-phase-0-integration-strategy.md)
(2026-08-16).** The Phase 0 audit and migration map are complete. Phases 0–2 may
begin. Epics 1, 3, 5, 6 and 8 may be decomposed once Epics 7, 9, 11 and 12 — which
describe already-shipped subsystems — are rescoped to formalize rather than rebuild
them. The role vocabulary is 14 snake_case (§65's 21-variant enum does not apply),
and the append-only JSONL ledger is the system of record with any §64 database as a
projection of it. Nothing else in this document is modified.

---

# 1. Executive Summary

AutoSpec shall evolve into a system capable of accepting human intent at multiple levels of abstraction and executing a complete, governed software-development lifecycle:

```text
Intent
  → Analysis
  → Planning
  → Specification
  → Work decomposition
  → Implementation
  → Testing
  → Independent review
  → UX validation
  → Documentation
  → Integration
  → Release
  → Monitoring
  → Maintenance
  → Continuous improvement
```

This evolution must not depend on a single model, provider, benchmark, or agent.

AutoSpec shall operate as an organization of specialized roles. Each role shall be assigned only to models that have demonstrated suitability for that purpose. Assignments must consider model capability, task type, risk, available context, modality, cost, latency, provider quota, local hardware, and historical performance.

Autonomy shall become a first-class property of:

- projects;
- repositories;
- goals;
- work items;
- roles;
- models;
- tools;
- actions;
- and workflows.

The system must maximize useful autonomy without sacrificing:

- independent verification;
- separation of duties;
- human authority;
- security boundaries;
- auditability;
- explainability;
- budget control;
- deterministic policy enforcement;
- and recoverability.

AutoSpec’s target state is not merely a collection of agents. It is a **governed autonomous engineering organization**.

---

# 2. Purpose

This specification defines the architecture and implementation requirements needed for AutoSpec to support progressively higher levels of autonomous software development.

It introduces four major foundational subsystems:

1. **Autonomy and Policy Engine**
2. **Capability, Benchmark, and Qualification Engine**
3. **Adaptive Model Routing Engine**
4. **Workflow and Organizational Orchestration Engine**

Supporting systems shall include:

- role management;
- task and risk classification;
- evidence collection;
- independent review;
- approval management;
- GitHub Projects and Kanban synchronization;
- audit logging;
- UX validation;
- documentation maintenance;
- release governance;
- budget enforcement;
- and operational dashboards.

---

# 3. Problem Statement

AI-assisted development commonly fails when autonomy is increased without equivalent improvements in governance.

Typical failure modes include:

- one model planning, implementing, and approving its own work;
- assigning models based on reputation rather than demonstrated purpose-specific capability;
- continuing to route work to providers that have exhausted quota or capacity;
- choosing the cheapest model even when it is not qualified;
- allowing coding benchmarks to qualify models for textual analysis or architecture;
- advancing workflows without machine-verifiable evidence;
- treating passing unit tests as proof that UI work is complete;
- losing traceability between requirements, changes, tests, and reviews;
- allowing retries to form unbounded loops;
- permitting agents to exceed cost, tool, or repository authority;
- making routing decisions that users cannot inspect;
- and granting project-level autonomy without reducing it for high-risk tasks.

AutoSpec shall solve these problems through explicit policy, role qualification, bounded authority, evidence-based workflow gates, and independent review.

---

# 4. Goals

AutoSpec shall provide:

1. A formal autonomy maturity model from A0 through A7.
2. Per-task effective-autonomy calculation.
3. Project, repository, branch, and work-item policy scopes.
4. Role-specific model qualification.
5. Purpose-specific benchmark integration.
6. Dynamic model and provider routing.
7. Usage-aware and quota-aware fallback.
8. First-class support for qualified local models.
9. Separation of planning, implementation, and review duties.
10. Structured workflow state and transition enforcement.
11. Human approval gates for sensitive operations.
12. Risk-adjusted testing and review requirements.
13. GitHub issue, pull request, milestone, and Project board integration.
14. Mandatory Kanban visualization for managed autonomous projects.
15. UX validation using vision and browser automation.
16. Documentation generation and drift detection.
17. Cost, token, latency, and retry budgets.
18. Append-only audit history.
19. Explainable routing and policy decisions.
20. Restart-safe and resumable workflows.
21. Goal-driven development at A6.
22. Controlled autonomous project improvement at A7.
23. Safe migration from current AutoSpec behavior.

---

# 5. Non-Goals

This specification does not authorize:

- unlimited self-modification;
- unbounded autonomous execution;
- automatic production deployment by default;
- automatic merging into protected branches by default;
- bypassing human approval for critical actions;
- using a single aggregate benchmark score for every role;
- treating model-reported confidence as proof of correctness;
- allowing agents to rewrite or disable their own governance policies;
- replacing deterministic validation with model opinion;
- or guaranteeing that autonomous work is defect-free.

AutoSpec shall remain a governed engineering system, not an unrestricted general-purpose autonomous agent.

---

# 6. Normative Language

The terms **SHALL**, **MUST**, **SHALL NOT**, and **MUST NOT** indicate mandatory requirements.

The terms **SHOULD** and **SHOULD NOT** indicate recommended behavior that may be changed only for a documented reason.

The term **MAY** indicates optional behavior.

---

# 7. Core Design Principles

## 7.1 Most restrictive authority wins

When multiple policies apply, AutoSpec shall use the most restrictive effective result.

## 7.2 Explicit deny overrides allow

An explicit prohibition shall override a lower-scope permission.

## 7.3 Models are untrusted workers

Model output shall be treated as untrusted input until validated.

## 7.4 Evidence outranks confidence

Passing tests, reproducible commands, structured review, and verified artifacts shall carry more authority than a model’s self-reported confidence.

## 7.5 Qualification precedes optimization

AutoSpec shall first eliminate unqualified models. It may optimize cost, speed, or convenience only among eligible models.

## 7.6 Implementation and approval remain independent

A model that implemented work shall not approve that work.

## 7.7 Autonomy is a ceiling, not a goal

A project configured for A6 does not require every task to execute at A6.

## 7.8 All important decisions must be explainable

Users shall be able to inspect why a task received its risk level, autonomy level, model assignment, approval requirement, and final disposition.

## 7.9 Workflows must be bounded

Retries, costs, tool invocations, context consumption, and execution time must have enforceable limits.

## 7.10 Autonomy must be reversible

Projects shall support pausing, lowering autonomy, canceling active work, and recovering from failed executions.

---

# 8. Conceptual Architecture

AutoSpec shall separate its control plane from its execution plane.

```text
┌──────────────────────────────────────────────────────────────┐
│                       Control Plane                          │
│                                                              │
│  Policy Engine     Risk Engine       Capability Registry     │
│  Role Registry     Model Router      Budget Controller       │
│  Orchestrator      Approval Engine   Audit Ledger            │
└─────────────────────────────┬────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                      Execution Plane                         │
│                                                              │
│  Planner Agent     Implementer Agent     Test Agent           │
│  Reviewer Agent    UX/Vision Agent       Documentation Agent  │
│  Security Agent    Release Agent         Tool Adapters        │
│                                                              │
│  Isolated worktrees / containers / browser sessions          │
└─────────────────────────────┬────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                     External Systems                         │
│                                                              │
│  Git / GitHub      CI/CD      Model Providers                │
│  Local Runtimes    Browser    Artifact Stores                │
│  Monitoring        Issue Trackers                            │
└──────────────────────────────────────────────────────────────┘
```

The control plane shall remain deterministic wherever practical.

Models shall not make final policy-enforcement decisions.

---

# 9. Operational Modes

AutoSpec shall support the following operational modes independently of autonomy level.

## 9.1 Advisory mode

AutoSpec calculates decisions and proposes actions but performs no mutations.

## 9.2 Shadow mode

AutoSpec runs routing, policy, and workflow calculations alongside existing behavior without enforcing them.

Shadow-mode results shall be recorded for comparison.

## 9.3 Enforced mode

AutoSpec enforces role qualification, policy, risk, budgets, and workflow gates.

## 9.4 Autonomous mode

AutoSpec executes permitted actions automatically within the configured autonomy ceiling.

A project may use A5 policy semantics while running in advisory or shadow mode.

---

# 10. AutoSpec Autonomy Levels

## A0 — Assistant

The human directs every meaningful action.

AutoSpec may:

- answer questions;
- inspect repository state;
- draft text;
- suggest commands;
- identify possible issues.

AutoSpec shall not mutate project state without an explicit action request.

---

## A1 — Task Executor

AutoSpec may execute a clearly defined task.

Example:

> Implement issue #431 according to the approved specification.

The human remains responsible for decomposition, sequencing, and acceptance.

---

## A2 — Workflow Executor

AutoSpec may execute a predefined workflow within an approved work item.

Example:

```text
Approved specification
  → implementation
  → tests
  → review
  → pull request
```

AutoSpec may determine intermediate execution steps, but it shall not independently redefine the work item’s goal.

---

## A3 — Managed Engineering Team

AutoSpec operates multiple specialized roles.

Required organizational capabilities include:

- planning;
- implementation;
- test planning;
- test execution;
- independent review;
- documentation;
- orchestration;
- and UX review where applicable.

Separation of duties becomes mandatory.

---

## A4 — Adaptive Engineering Organization

AutoSpec selects models and providers dynamically based on measured suitability.

Required capabilities include:

- role qualification;
- benchmark integration;
- usage-aware fallback;
- provider-health checks;
- local-model participation;
- cost and latency optimization;
- modality-aware routing;
- model-version tracking;
- and escalation when no qualified model is available.

---

## A5 — Self-Managing Project

AutoSpec manages the operational development lifecycle.

Capabilities include:

- backlog maintenance;
- issue creation;
- issue status updates;
- dependency management;
- milestones;
- GitHub Projects;
- Kanban state;
- pull requests;
- documentation lifecycle;
- technical-debt tracking;
- release preparation;
- and stalled-work recovery.

Humans retain control over strategy, policy, budget, and restricted actions.

---

## A6 — Goal-Driven Development

A human may provide an outcome instead of a predefined issue.

Example:

> Add enterprise authentication to AutoSpec.

AutoSpec may determine:

- research tasks;
- architectural alternatives;
- specifications;
- milestones;
- issue decomposition;
- dependency ordering;
- implementation sequencing;
- test strategy;
- migration plans;
- documentation;
- and rollout strategy.

Policy may still require human approval for the goal plan or selected architecture.

---

## A7 — Autonomous Software Organization

AutoSpec continuously operates the project within policy.

AutoSpec may:

- detect defects;
- identify technical debt;
- propose improvements;
- create work;
- prioritize approved classes of work;
- implement changes;
- run validation;
- update documentation;
- prepare releases;
- monitor regressions;
- and maintain its own engineering backlog.

AutoSpec shall not independently expand its authority, raise its autonomy ceiling, disable controls, or approve changes to its governance system.

---

# 11. Effective Autonomy

AutoSpec shall calculate autonomy separately for every work item.

Conceptually:

```text
effective_autonomy =
    minimum(
        organization_limit,
        project_limit,
        repository_limit,
        branch_limit,
        work_item_limit,
        risk_limit,
        data_classification_limit,
        role_limit,
        model_limit,
        tool_limit,
        budget_limit
    )
```

The result shall be recorded with an explanation.

Example:

```yaml
project_autonomy: A6
repository_autonomy: A5
risk_autonomy: A2
model_autonomy: A4
effective_autonomy: A2

reasons:
  - authentication boundary affected
  - human architecture approval required
```

AutoSpec shall never silently elevate effective autonomy.

---

# 12. Policy Hierarchy

Policies shall support the following scopes:

1. Organization
2. Project
3. Repository
4. Branch or environment
5. Goal
6. Work item
7. Role
8. Tool
9. Execution attempt

The default order shall be most-restrictive-wins.

Policies may tighten authority at lower scopes but shall not loosen an organization-level hard restriction without an authorized override.

Every override shall include:

- actor;
- timestamp;
- reason;
- affected scope;
- previous value;
- new value;
- expiration, where applicable;
- and audit event ID.

Arbitrary executable code shall not be allowed in policy configuration for the initial implementation.

---

# 13. Risk Classification

Every goal and work item shall receive a risk classification before execution.

Required levels:

```text
LOW
MODERATE
HIGH
CRITICAL
```

Risk evaluation shall consider:

- security boundaries;
- authentication;
- authorization;
- secrets;
- encryption;
- personal or sensitive data;
- destructive database operations;
- schema migration complexity;
- public API compatibility;
- production infrastructure;
- financial or billing behavior;
- dependency changes;
- repository size;
- test coverage;
- blast radius;
- novelty;
- reversibility;
- model confidence;
- benchmark coverage;
- and expected user impact.

## 13.1 Default risk examples

### Low

- documentation typo;
- non-functional formatting;
- isolated internal refactor with complete tests;
- test-only changes.

### Moderate

- ordinary feature development;
- internal API change;
- new dependency with no elevated privileges;
- moderate UI behavior changes.

### High

- authentication;
- authorization;
- data migration;
- public API breakage;
- privileged tool access;
- external payment or billing integration;
- release infrastructure.

### Critical

- production credential handling;
- destructive production action;
- security-boundary redesign;
- irreversible migration;
- automatic release authority;
- governance or autonomy-policy modification.

Risk classifications shall be deterministic where rules exist and reviewable where inference is involved.

---

# 14. Data and Action Classification

AutoSpec shall classify both information and actions.

## 14.1 Data classes

```text
PUBLIC
INTERNAL
CONFIDENTIAL
RESTRICTED
SECRET
```

## 14.2 Action classes

```text
READ_ONLY
LOCAL_MUTATION
REPOSITORY_MUTATION
REMOTE_MUTATION
DESTRUCTIVE
PRODUCTION
GOVERNANCE
```

Model providers and tools shall declare which classes they are permitted to receive or perform.

A task involving restricted or secret data shall not be routed to a provider that is not approved for that class.

---

# 15. Work-Item Model

Every executable unit shall be represented as a work item.

Required fields include:

```yaml
id: string
project_id: string
repository_id: string
parent_goal_id: optional-string

title: string
description: string
task_kind: string

status: string
risk: string
requested_autonomy: string
effective_autonomy: string

requirements:
  - requirement-id

dependencies:
  - work-item-id

required_roles:
  - role-id

required_capabilities:
  capability-name: minimum-score

data_classification: string
action_classification: string

budget_id: string
policy_snapshot_id: string

created_at: timestamp
updated_at: timestamp
```

A work item shall be immutable in identity but versioned in content.

Substantial scope changes shall create a new revision and may require renewed approval.

---

# 16. Role Taxonomy

AutoSpec shall provide first-class role definitions.

## 16.1 Governance and coordination roles

- `ORCHESTRATOR`
- `RISK_ASSESSOR`
- `ROUTING_ADVISOR`
- `BENCHMARK_ANALYST`
- `POLICY_ADVISOR`

## 16.2 Discovery and planning roles

- `RESEARCHER`
- `REQUIREMENTS_ANALYST`
- `ARCHITECT`
- `PLANNER`
- `SPECIFICATION_WRITER`

## 16.3 Execution roles

- `IMPLEMENTER`
- `TEST_EXECUTOR`
- `DOCUMENTATION_WRITER`
- `MIGRATION_EXECUTOR`
- `RELEASE_PREPARER`

## 16.4 Assurance roles

- `TEST_PLANNER`
- `CODE_REVIEWER`
- `SECURITY_REVIEWER`
- `DEPENDENCY_REVIEWER`
- `UX_REVIEWER`
- `ACCESSIBILITY_REVIEWER`
- `INTEGRATION_REVIEWER`
- `RELEASE_REVIEWER`

A project may define additional roles, but custom roles must declare their required capabilities, authority, and allowed tool classes.

---

# 17. Role Responsibilities

## 17.1 Orchestrator

The orchestrator shall:

- decompose work;
- assign roles;
- request model routing;
- manage workflow state;
- enforce dependencies;
- enforce policy;
- enforce budgets;
- schedule retries;
- request approvals;
- and escalate blocked work.

The orchestrator should not implement application code.

## 17.2 Planner

The planner shall:

- interpret approved goals;
- identify dependencies;
- define work sequencing;
- identify unknowns;
- estimate risk;
- and produce a machine-readable plan.

## 17.3 Implementer

The implementer shall:

- modify only authorized files and systems;
- follow the approved specification;
- add or update tests;
- report assumptions;
- and produce implementation evidence.

## 17.4 Test planner

The test planner shall create a test strategy independently from the implementation.

## 17.5 Reviewer

The reviewer shall evaluate:

- requirement satisfaction;
- correctness;
- maintainability;
- regressions;
- tests;
- failure handling;
- and architectural fit.

## 17.6 UX reviewer

The UX reviewer shall inspect actual rendered output and interactions, not merely source code.

## 17.7 Documentation writer

The documentation writer shall identify and update all affected documentation surfaces.

---

# 18. Structured Role Contracts

Each role shall produce a validated structured result.

Free-form prose may accompany the result but shall not replace it.

Example review contract:

```yaml
role: CODE_REVIEWER
work_item_id: WI-431
verdict: PASS
confidence: 0.92

requirement_results:
  REQ-431-01: PASS
  REQ-431-02: PASS

findings:
  critical: []
  major: []
  minor:
    - id: FIND-1
      description: Improve error message wording
      file: src/router.rs
      line: 218

evidence:
  - evidence-id-1
  - evidence-id-2

recommended_transition: DOCUMENTING
```

Invalid structured output shall be rejected and may be retried within policy limits.

---

# 19. Separation of Duties

For the same work item:

```text
Planner != Implementer
Implementer != Final Reviewer
Implementer != Security Reviewer
```

Identity shall be evaluated using the underlying model fingerprint, not merely the agent session.

A model fingerprint shall include, where available:

- provider;
- model family;
- exact model version;
- deployment identifier;
- quantization;
- and runtime profile.

A sufficiently qualified model may act as both planner and reviewer when:

- it did not implement the work;
- project policy permits the combination;
- and risk level does not require stronger independence.

For high-risk and critical work, policy should require the final reviewer to use a different provider or independently hosted model family when available.

When independent qualified review is unavailable, AutoSpec shall escalate rather than silently waive the requirement.

---

# 20. Provider and Model Identity

AutoSpec shall distinguish between:

- provider;
- model family;
- model version;
- deployment;
- local runtime;
- quantization;
- hardware profile;
- context configuration;
- and tool configuration.

Example:

```yaml
provider: local-llama-cpp
family: qwen-coder
version: 3.8-27b
deployment: workstation-bender
quantization: bf16
context_window: 65536
hardware_profile: rtx-5090
```

Different quantizations or runtime configurations shall be treated as distinct benchmarkable runtime profiles.

---

# 21. Capability Taxonomy

The capability registry shall support, at minimum:

## 21.1 General capabilities

- instruction following;
- structured-output compliance;
- tool use;
- long-context handling;
- uncertainty recognition;
- error recovery.

## 21.2 Analysis capabilities

- textual analysis;
- requirements interpretation;
- argument analysis;
- ambiguity detection;
- research synthesis;
- repository reasoning.

## 21.3 Planning capabilities

- architecture reasoning;
- task decomposition;
- dependency analysis;
- risk identification;
- migration planning;
- test planning.

## 21.4 Engineering capabilities

- general coding;
- Rust implementation;
- debugging;
- refactoring;
- concurrency;
- API design;
- database work;
- frontend implementation;
- infrastructure code.

## 21.5 Assurance capabilities

- code review;
- defect detection;
- security review;
- dependency review;
- test adequacy review;
- architecture conformance;
- release review.

## 21.6 Communication capabilities

- technical documentation;
- user documentation;
- API documentation;
- changelog generation;
- concise issue writing.

## 21.7 Modal capabilities

- vision;
- UI evaluation;
- screenshot interpretation;
- image generation;
- audio, when supported.

Scores shall be role- and task-specific rather than collapsed into a single universal score.

---

# 22. Capability Registry

Each model runtime profile shall have a capability record.

```yaml
model_profile_id: model-profile-123

capabilities:
  rust_implementation:
    score: 93
    confidence: 0.91
    source: benchmark
    benchmark_version: rust-suite-4
    measured_at: 2026-08-10

  textual_analysis:
    score: 61
    confidence: 0.84
    source: benchmark
    benchmark_version: analysis-suite-2
    measured_at: 2026-08-10

role_qualification:
  IMPLEMENTER:
    status: QUALIFIED
    max_autonomy: A4

  ARCHITECT:
    status: NOT_QUALIFIED

  CODE_REVIEWER:
    status: CONDITIONAL
    max_risk: MODERATE
```

Manual scores shall be marked as manual overrides and shall not appear equivalent to measured benchmark evidence.

---

# 23. Benchmark System

The benchmark system shall evolve from model comparison into model qualification.

Benchmarks shall measure both quality and operating characteristics.

## 23.1 Required benchmark dimensions

- correctness;
- requirement adherence;
- compilation success;
- test success;
- defect rate;
- review accuracy;
- false-positive review rate;
- structured-output validity;
- latency;
- time to first token;
- generation throughput;
- context reliability;
- token usage;
- monetary cost;
- retry rate;
- tool-call success;
- and outcome acceptance.

## 23.2 Purpose-specific benchmark suites

At minimum:

- Rust implementation;
- repository exploration;
- bug diagnosis;
- test generation;
- test planning;
- textual analysis;
- specification interpretation;
- architecture;
- code review;
- security review;
- documentation;
- UX screenshot review;
- browser-interaction evaluation;
- long-context retrieval;
- and tool-use reliability.

## 23.3 Benchmark integrity

Benchmark suites shall be versioned.

Results shall include:

- dataset version;
- task IDs;
- evaluator version;
- model profile;
- runtime configuration;
- hardware;
- date;
- and raw evidence.

Benchmarks shall not rely only on model self-grading.

---

# 24. Role Qualification

Role qualification shall use hard requirements and optional weighted scoring.

Example:

```yaml
roles:
  ARCHITECT:
    hard_requirements:
      architecture_reasoning: 88
      textual_analysis: 82
      repository_reasoning: 80
      structured_output: 85

    weighted_score:
      architecture_reasoning: 0.35
      textual_analysis: 0.25
      repository_reasoning: 0.20
      risk_identification: 0.20

    minimum_weighted_score: 86
```

A model may be:

```text
QUALIFIED
CONDITIONAL
NOT_QUALIFIED
STALE
SUSPENDED
```

Conditional qualification shall state its constraints, such as:

- maximum risk;
- maximum autonomy;
- limited repository size;
- required secondary reviewer;
- or prohibited tool classes.

---

# 25. Model Drift and Requalification

Qualification shall be time-sensitive.

A model profile shall be re-evaluated when any of the following changes:

- model version;
- provider deployment;
- system prompt;
- tool configuration;
- quantization;
- context settings;
- local hardware;
- inference runtime;
- benchmark version;
- or observed performance.

AutoSpec shall support configurable qualification expiration.

Example:

```yaml
qualification:
  maximum_age_days: 90
  require_rebenchmark_on_model_version_change: true
  require_rebenchmark_on_quantization_change: true
```

Stale qualifications may be used only if policy explicitly permits them.

---

# 26. Runtime Health, Capacity, and Quota

Before assigning work, AutoSpec shall query or estimate provider status.

Required status fields include:

```yaml
provider_status:
  reachable: true
  authenticated: true
  quota_available: true
  rate_limited: false
  current_concurrency: 2
  maximum_concurrency: 8
  estimated_queue_delay_ms: 400
  status_age_seconds: 12
```

AutoSpec shall account for:

- provider outages;
- exhausted usage;
- rate limits;
- local VRAM constraints;
- runtime queue depth;
- model load time;
- context limits;
- and tool availability.

Stale health information shall not be treated as current indefinitely.

---

# 27. Adaptive Model Routing

The router shall first construct an eligible candidate set.

A candidate is eligible only when it satisfies:

- required role qualification;
- minimum capabilities;
- maximum task risk;
- effective autonomy requirement;
- modality requirement;
- tool requirement;
- context requirement;
- data-classification policy;
- separation-of-duties policy;
- provider availability;
- and hard budget constraints.

Only eligible models may be ranked.

A conceptual routing interface:

```rust
pub trait ModelRouter {
    async fn select(
        &self,
        request: RoutingRequest,
    ) -> Result<RoutingDecision, RoutingError>;
}
```

The routing decision shall contain:

```rust
pub struct RoutingDecision {
    pub selected_profile_id: ModelProfileId,
    pub eligible_candidates: Vec<CandidateScore>,
    pub rejected_candidates: Vec<RejectedCandidate>,
    pub explanation: RoutingExplanation,
    pub policy_snapshot_id: PolicySnapshotId,
}
```

---

# 28. Routing Optimization

Among eligible candidates, AutoSpec may optimize:

- predicted success;
- historical first-pass acceptance;
- cost;
- latency;
- availability;
- context fit;
- local execution preference;
- privacy;
- retry probability;
- and provider diversity.

Conceptually:

```text
utility =
    quality_weight × predicted_success
  + availability_weight × availability
  + privacy_weight × privacy_fit
  - cost_weight × expected_cost
  - latency_weight × expected_latency
  - retry_weight × expected_retry_cost
```

The exact formula shall be configurable and versioned.

Deterministic tie-breaking shall be used.

---

# 29. Usage-Aware Fallback

AutoSpec shall support ordered and dynamic fallback.

Example:

```text
Preferred implementer: Codex
Status: usage exhausted

Fallback candidate: Claude
Role qualification: qualified
Risk limit: acceptable
Capacity: available

Decision: assign Claude
```

Fallback shall never bypass role qualification.

If no eligible fallback exists, AutoSpec shall:

1. mark the work blocked;
2. create an escalation;
3. state which constraints eliminated each candidate;
4. and avoid assigning an unqualified model merely to continue.

---

# 30. Local-Model Policy

Qualified local models shall be first-class candidates.

AutoSpec may prefer local execution for:

- lower cost;
- privacy;
- low latency;
- high throughput;
- or provider independence.

Local models shall be subject to the same qualification and evidence requirements as hosted models.

Local benchmark records shall include:

- CPU;
- GPU;
- VRAM;
- RAM;
- inference engine;
- engine version;
- quantization;
- batch size;
- context size;
- KV-cache configuration;
- concurrency;
- and generation parameters.

Recommended concurrency benchmark points:

```text
1, 2, 4, 8, 16, 32, 64
```

Only feasible values need to be executed.

---

# 31. Modality-Aware Routing

Models shall declare supported modalities:

```text
TEXT
CODE
VISION
IMAGE_GENERATION
AUDIO
```

Tasks shall declare required modalities.

Examples:

- source-code implementation may require text and code;
- screenshot validation requires vision;
- graphical asset creation requires image generation;
- interactive UX validation requires vision plus browser tools.

A text-only model shall not be the sole assurance agent for visual UI work.

---

# 32. Context Management

AutoSpec shall build role-specific context packages.

An implementer should not automatically receive every planning discussion, benchmark log, and unrelated repository document.

Context packages shall include only relevant:

- requirements;
- approved specification;
- affected repository regions;
- architecture decisions;
- dependency information;
- test expectations;
- policy constraints;
- and prior findings.

Context packages shall be versioned and hashed.

AutoSpec shall record which context package was used for each execution attempt.

Repository-provided instructions shall be treated as potentially untrusted and shall not override AutoSpec policy.

---

# 33. Tool Permissions and Execution Isolation

Agents shall receive only the minimum required tool authority.

Tool permissions shall be role-specific.

Example:

```yaml
IMPLEMENTER:
  allowed:
    - repository_read
    - worktree_write
    - local_build
    - local_test

  denied:
    - protected_branch_push
    - production_deploy
    - secret_read
    - policy_write
```

Execution should occur in isolated:

- Git worktrees;
- temporary branches;
- containers;
- browser profiles;
- and temporary directories.

The implementation shall support:

- command allowlists;
- command denylists;
- filesystem boundaries;
- network restrictions;
- timeout enforcement;
- output-size limits;
- and process cancellation.

Direct mutation of protected branches shall be prohibited by default.

Force pushing shall be prohibited by default.

---

# 34. Privacy and Secrets

Secrets shall never be placed directly in prompts unless explicitly allowed by policy.

AutoSpec shall use secret references and scoped injection.

Secret access events shall be audited.

Models and providers shall declare whether they are approved for:

- public data;
- internal data;
- confidential data;
- restricted data;
- and secrets.

Potential secret material detected in model output shall be redacted from ordinary logs.

AutoSpec shall support repository and organization-level provider restrictions.

---

# 35. Orchestrator

The orchestrator shall manage the engineering organization.

Responsibilities include:

- goal intake;
- task classification;
- risk calculation;
- effective-autonomy calculation;
- plan execution;
- role assignment;
- model routing;
- dependency scheduling;
- workflow transitions;
- evidence verification;
- retry management;
- approval requests;
- GitHub synchronization;
- and escalation.

The orchestrator shall not independently reinterpret an approved requirement after implementation has started without producing a revision.

All orchestrator decisions shall produce structured events.

---

# 36. Workflow Engine

Workflows shall be represented as explicit state machines or DAGs.

Required states include:

```text
DISCOVERED
TRIAGED
ANALYZING
PLANNING
SPECIFYING
SPEC_REVIEW
READY
IMPLEMENTING
BUILDING
TESTING
REVIEWING
SECURITY_REVIEW
UX_REVIEW
DOCUMENTING
INTEGRATION_REVIEW
WAITING_FOR_APPROVAL
READY_TO_MERGE
MERGING
RELEASE_PREPARATION
RELEASE_REVIEW
DONE
BLOCKED
ESCALATED
CANCELED
FAILED
```

Not every state applies to every work item.

Transitions shall be controlled by policy and evidence.

Agents shall request transitions; the workflow engine shall authorize them.

---

# 37. Validation Evidence

Every meaningful stage shall produce evidence.

Evidence types include:

- command execution;
- exit code;
- build result;
- test result;
- coverage result;
- linter result;
- benchmark result;
- screenshot;
- browser trace;
- reviewer verdict;
- security scan;
- dependency scan;
- documentation diff;
- API compatibility report;
- migration rehearsal;
- and artifact hash.

Evidence records shall include:

```yaml
id: evidence-id
type: test-result
work_item_id: WI-431
attempt_id: ATT-7
producer_role: TEST_EXECUTOR
producer_model: model-profile-2
created_at: timestamp
content_hash: sha256
artifact_location: string
summary: string
```

Required gates shall not pass merely because an agent claims they passed.

---

# 38. Planning and Specification Pipeline

The planning pipeline shall:

1. normalize the human goal or request;
2. identify ambiguity;
3. identify repository and system context;
4. classify risk;
5. propose architecture where needed;
6. decompose the goal into a dependency DAG;
7. define stable requirement IDs;
8. define acceptance criteria;
9. define test expectations;
10. define documentation impact;
11. define rollout and migration requirements;
12. and request approval where policy requires it.

Specifications shall have lifecycle states:

```text
DRAFT
IN_REVIEW
APPROVED
IMPLEMENTING
VERIFIED
SUPERSEDED
REJECTED
```

Implementation shall target an approved specification revision.

---

# 39. Implementation Pipeline

The implementer shall receive:

- approved requirement IDs;
- approved specification revision;
- authorized files or repository scope;
- relevant architecture decisions;
- test expectations;
- policy constraints;
- and budget limits.

The implementation result shall include:

- changed files;
- added files;
- deleted files;
- commands run;
- tests added or updated;
- assumptions;
- unresolved questions;
- and known limitations.

The implementer shall not mark its own work accepted.

---

# 40. Testing Pipeline

Testing shall be planned independently from implementation for A3 and above.

The test planner shall determine which test types apply:

- unit;
- integration;
- contract;
- end-to-end;
- regression;
- property-based;
- fuzz;
- concurrency;
- performance;
- security;
- migration;
- browser;
- accessibility;
- and visual-regression tests.

Risk shall influence test depth.

Critical behavior shall not rely exclusively on model-generated assertions without deterministic execution.

Test results shall map back to requirement IDs.

---

# 41. Code Review

Code review shall evaluate:

- requirement satisfaction;
- correctness;
- edge cases;
- failure behavior;
- maintainability;
- architectural consistency;
- test adequacy;
- observability;
- performance implications;
- compatibility;
- and documentation impact.

Review findings shall be classified:

```text
CRITICAL
MAJOR
MINOR
SUGGESTION
```

Critical and major findings shall block acceptance unless an authorized waiver exists.

Review confidence shall be recorded but shall not replace findings and evidence.

---

# 42. Security and Supply-Chain Review

Security review shall be automatically required when work affects:

- authentication;
- authorization;
- cryptography;
- secret handling;
- network exposure;
- file permissions;
- code execution;
- dependency management;
- production deployment;
- or restricted data.

Supply-chain review shall include, where supported:

- dependency provenance;
- vulnerability scanning;
- license checks;
- lockfile review;
- unexpected transitive dependencies;
- build-script review;
- and artifact integrity.

A general code reviewer shall not automatically satisfy a required security-review role.

---

# 43. UX, Vision, and Automation Review

UI-affecting work shall require actual rendered validation.

The UX pipeline should support:

- browser automation;
- screenshots;
- expected-versus-actual comparison;
- viewport testing;
- overflow detection;
- clipping detection;
- alignment inspection;
- contrast checks;
- keyboard-navigation checks;
- interaction testing;
- loading-state inspection;
- error-state inspection;
- and responsive-layout inspection.

The UX reviewer shall receive screenshots or browser traces.

A UI work item shall not be accepted solely because source-code tests pass.

Where a visual baseline exists, AutoSpec shall record visual differences and the disposition of those differences.

---

# 44. Image and Graphic Generation

When a work item requires generated visual assets, AutoSpec shall use a model qualified for image generation.

Generated assets shall be reviewed for:

- requirement compliance;
- dimensions;
- format;
- transparency;
- accessibility;
- licensing or provenance constraints;
- consistency with the design system;
- and unintended text or artifacts.

The model generating an asset shall not be the sole UX reviewer of that asset.

---

# 45. Documentation Agent

The documentation writer shall inspect the change and determine affected documentation surfaces.

Possible outputs include:

- README;
- user guide;
- configuration reference;
- CLI help;
- API documentation;
- architecture documentation;
- ADR;
- migration guide;
- troubleshooting guide;
- changelog;
- release notes;
- and examples.

Documentation review shall verify that examples and commands are valid where feasible.

Documentation shall be linked to the requirement and implementation that caused the change.

---

# 46. Architecture Conformance

AutoSpec shall maintain machine-readable architecture rules where possible.

Examples:

- forbidden dependency directions;
- crate boundaries;
- module ownership;
- public API restrictions;
- persistence boundaries;
- provider abstraction requirements;
- and layering rules.

Architecture checks may be deterministic, model-assisted, or both.

Model-assisted architecture review shall produce specific evidence and shall not override deterministic failures.

Material architectural changes shall create or update an ADR.

---

# 47. Requirement Traceability

Every implementation work item shall support traceability:

```text
Goal
  → specification
  → requirement IDs
  → issue or work item
  → changed files
  → tests
  → review findings
  → documentation
  → pull request
  → release
```

A requirement shall be marked verified only when linked evidence exists.

AutoSpec shall identify:

- requirements with no implementation;
- implementation with no requirement;
- requirements with no tests;
- and user-facing changes with no documentation disposition.

---

# 48. Human Approval Gates

Supported approval gates shall include:

```text
GOAL_APPROVAL
PLAN_APPROVAL
SPEC_APPROVAL
ARCHITECTURE_APPROVAL
SECURITY_APPROVAL
DATA_MIGRATION_APPROVAL
DEPENDENCY_APPROVAL
MERGE_APPROVAL
RELEASE_APPROVAL
PRODUCTION_APPROVAL
GOVERNANCE_CHANGE_APPROVAL
```

Approval requirements shall be configurable by risk and autonomy level.

An approval record shall contain:

- approver identity;
- authority scope;
- decision;
- timestamp;
- specification or artifact revision;
- reason;
- and expiration where applicable.

An approval of one revision shall not automatically approve a materially changed revision.

---

# 49. Escalation and Disagreement Resolution

AutoSpec shall escalate when:

- no qualified model exists;
- qualified providers are unavailable;
- model outputs materially disagree;
- review confidence is too low;
- evidence is contradictory;
- a specification remains ambiguous;
- retries are exhausted;
- a security-sensitive change is detected;
- budget is insufficient;
- or policy requires human judgment.

Escalation targets may include:

- a stronger model;
- an additional independent reviewer;
- a domain specialist;
- an architecture arbiter;
- a security arbiter;
- or a human.

Disagreement shall not be resolved by simple majority vote alone.

The arbiter shall compare claims against requirements and evidence.

---

# 50. Retry Limits and Loop Prevention

All autonomous cycles shall be bounded.

Example:

```yaml
retry_limits:
  structured_output_repair: 2
  implementation_review_cycles: 3
  test_fix_cycles: 3
  architecture_replans: 2
  provider_failover_attempts: 3
```

A retry shall create a new attempt record.

AutoSpec shall detect repeated equivalent failures.

When the same failure recurs without meaningful state change, the workflow shall escalate instead of consuming the remaining retry budget blindly.

---

# 51. Concurrency, Locking, and Resumability

AutoSpec shall support multiple concurrent work items while preventing conflicting changes.

Required mechanisms include:

- dependency-aware scheduling;
- repository concurrency limits;
- isolated branches or worktrees;
- resource locks;
- overlapping-file detection;
- merge-conflict detection;
- and stale-base detection.

Workflow state shall be persisted after every transition.

After restart, AutoSpec shall:

- reconstruct active workflows;
- identify interrupted external processes;
- avoid duplicate mutations;
- and resume or safely fail the attempt.

Mutating operations shall support idempotency keys where feasible.

---

# 52. GitHub Integration

At A5 and above, GitHub integration shall be a required part of project operation unless the project is explicitly configured for a different supported source-control platform.

Capabilities shall include:

- issue creation;
- issue updates;
- issue dependencies;
- labels;
- milestones;
- assignees or role metadata;
- pull-request creation;
- draft pull requests;
- review status;
- evidence attachment or linking;
- check-run status;
- and release linkage.

AutoSpec shall not push directly to protected default branches.

Pull requests shall remain draft or blocked until mandatory gates pass.

---

# 53. Mandatory Kanban and GitHub Projects

A5 projects shall require a linked GitHub Project board.

AutoSpec shall create or reconcile the board idempotently.

Required statuses:

```text
Backlog
Discovered
Analysis
Planning
Specification Review
Ready
Implementation
Testing
Code Review
Security Review
UX Review
Documentation
Blocked
Approval Required
Ready to Merge
Release Preparation
Done
```

The board shall expose fields for:

- risk;
- requested autonomy;
- effective autonomy;
- work-item type;
- current role;
- assigned model;
- milestone;
- dependency status;
- review status;
- approval status;
- attempt count;
- cost;
- and evidence link.

Workflow transitions shall synchronize to the board.

Manual board changes shall be reconciled according to project policy rather than overwritten blindly.

---

# 54. Goal-Driven Development at A6

An A6 goal shall produce a goal plan containing:

- goal statement;
- success metrics;
- constraints;
- assumptions;
- architecture options;
- selected approach;
- risk analysis;
- work-item DAG;
- milestones;
- estimated cost;
- estimated model usage;
- approval requirements;
- release strategy;
- and rollback strategy.

Before implementation, AutoSpec shall verify that the goal plan is internally consistent and that all required approval gates are satisfied.

Large goals shall be decomposed into epics and work items.

AutoSpec shall prevent circular dependencies.

---

# 55. Autonomous Discovery at A7

At A7, AutoSpec may discover candidate work from:

- failing tests;
- flaky tests;
- TODO and FIXME markers;
- security advisories;
- outdated dependencies;
- benchmark regressions;
- performance regressions;
- documentation drift;
- dead code;
- architecture violations;
- recurring support issues;
- build failures;
- and repeated review findings.

Discovered work shall first enter a proposal state.

Policy shall define which categories may be automatically promoted into execution.

AutoSpec shall not autonomously:

- raise its own autonomy;
- change governance hard limits;
- grant itself new tools;
- remove approval requirements;
- modify audit history;
- or suppress discovered security findings.

Changes to AutoSpec’s own governance components shall be treated as critical risk.

---

# 56. Release Management

Release preparation shall include, where applicable:

- version calculation;
- changelog generation;
- release notes;
- migration documentation;
- compatibility analysis;
- artifact generation;
- SBOM generation;
- checksums;
- signing;
- packaging;
- and release-candidate validation.

Release approval shall be independent from implementation.

Automatic release shall be disabled by default.

A project must explicitly configure release authority and environments.

---

# 57. Post-Release Monitoring and Rollback

AutoSpec shall support post-release validation through adapters.

Possible signals include:

- CI status;
- deployment status;
- error rate;
- failed health checks;
- performance regression;
- crash reports;
- user-reported issues;
- and rollback events.

Every release shall record:

- source commit;
- specification set;
- artifact hashes;
- deployment target;
- approval records;
- and rollback instructions.

For reversible Git changes, AutoSpec should be able to prepare a revert pull request.

Production rollback execution shall require explicit authority.

---

# 58. Audit Ledger

AutoSpec shall maintain an append-only audit ledger.

Audit events shall include:

- policy decisions;
- autonomy calculations;
- risk calculations;
- role assignments;
- routing decisions;
- provider fallbacks;
- tool grants;
- commands;
- workflow transitions;
- approvals;
- overrides;
- retries;
- cancellations;
- GitHub mutations;
- merge decisions;
- and release actions.

Each event shall contain:

```yaml
event_id: string
sequence: integer
event_type: string
timestamp: timestamp
actor_type: human-or-agent
actor_id: string
work_item_id: optional-string
attempt_id: optional-string
policy_snapshot_id: optional-string
payload: object
```

A tamper-evident hash chain should be supported for higher-assurance deployments.

Audit entries shall not contain unredacted secrets.

---

# 59. Explainability

Users shall be able to ask:

- Why was this autonomy level selected?
- Why was this task classified high risk?
- Why was this model selected?
- Why was another model rejected?
- Why did AutoSpec fall back to another provider?
- Why is this work blocked?
- Which approval is missing?
- Which evidence failed?
- Why was the workflow retried?
- How much has this work cost?
- Which requirement remains unverified?

The answer shall be derived from stored policy, routing, evidence, and audit records rather than reconstructed from model memory.

---

# 60. Budget Governance

Budgets shall support:

- monetary cost;
- tokens;
- compute time;
- wall-clock duration;
- tool calls;
- provider-specific usage;
- local GPU time;
- retries;
- and concurrent tasks.

Budgets may exist at:

- organization;
- project;
- goal;
- milestone;
- work item;
- provider;
- and time-window scopes.

Budgets shall support:

```text
SOFT_LIMIT
HARD_LIMIT
```

A soft limit generates warning or approval.

A hard limit blocks new spending unless an authorized override exists.

Budget exhaustion shall not cause AutoSpec to select an unqualified model.

---

# 61. Performance and Self-Evaluation

AutoSpec shall track outcome metrics by model, role, task type, repository, and risk level.

Required metrics include:

- first-pass acceptance rate;
- eventual acceptance rate;
- review rejection rate;
- human rejection rate;
- escaped defect rate;
- rollback rate;
- average retries;
- average completion time;
- cost per accepted issue;
- tokens per accepted issue;
- escalation rate;
- provider-failure rate;
- route-change rate;
- and requirement-verification rate.

Historical results shall feed routing predictions.

Self-reported model confidence shall be calibrated against observed outcomes before it influences routing significantly.

---

# 62. Dashboards

The AutoSpec interface shall provide:

## 62.1 Project autonomy dashboard

- configured autonomy;
- effective autonomy distribution;
- active workflows;
- blocked work;
- pending approvals;
- budget usage;
- and recent escalations.

## 62.2 Model capability dashboard

- model profiles;
- benchmark scores;
- role qualifications;
- stale qualifications;
- historical acceptance;
- cost;
- latency;
- and runtime health.

## 62.3 Work-item detail

- requirements;
- risk;
- policy decision;
- assigned roles;
- assigned models;
- workflow timeline;
- attempts;
- evidence;
- findings;
- GitHub links;
- cost;
- and approval state.

## 62.4 Organization health

- backlog;
- work in progress;
- review bottlenecks;
- provider capacity;
- repeated failures;
- model drift;
- and release readiness.

---

# 63. CLI and API

The exact syntax may align with existing AutoSpec conventions, but equivalent capabilities shall exist.

Recommended CLI commands:

```text
autospec autonomy status
autospec autonomy set A4
autospec autonomy explain <work-item>

autospec models list
autospec models inspect <model-profile>
autospec models qualify <model-profile>
autospec models benchmark <model-profile>

autospec route explain <work-item>
autospec work start <work-item>
autospec work pause <work-item>
autospec work resume <work-item>
autospec work cancel <work-item>

autospec approvals list
autospec approvals approve <approval-id>
autospec approvals reject <approval-id>

autospec project reconcile-github
autospec audit show <work-item>
autospec emergency-stop
```

Recommended API surfaces:

```text
GET    /v1/autonomy/status
POST   /v1/autonomy/evaluate
GET    /v1/models
GET    /v1/models/{id}
POST   /v1/benchmarks
POST   /v1/routes/evaluate
GET    /v1/routes/{id}/explanation
POST   /v1/work-items
GET    /v1/work-items/{id}
POST   /v1/work-items/{id}/start
POST   /v1/work-items/{id}/pause
POST   /v1/work-items/{id}/cancel
POST   /v1/approvals/{id}/decision
GET    /v1/audit
POST   /v1/emergency-stop
```

Mutating API operations shall support authentication, authorization, and idempotency.

---

# 64. Persistence Model

AutoSpec shall use a storage abstraction.

If the repository does not already provide an appropriate persistence system, the default implementation should use SQLite through a Rust-native database layer, with a migration path to PostgreSQL.

Minimum logical tables or aggregates:

- organizations;
- projects;
- repositories;
- branches;
- policy_documents;
- policy_snapshots;
- goals;
- work_items;
- work_item_revisions;
- dependencies;
- roles;
- role_requirements;
- providers;
- model_profiles;
- model_versions;
- capability_scores;
- benchmark_runs;
- benchmark_results;
- role_qualifications;
- provider_health;
- routing_decisions;
- role_assignments;
- execution_attempts;
- workflow_transitions;
- evidence_records;
- review_findings;
- approvals;
- budgets;
- budget_usage;
- GitHub links;
- releases;
- metrics;
- and audit_events.

Binary artifacts should be content-addressed and stored outside ordinary relational rows, with hashes referenced from the database.

---

# 65. Core Rust Types

Conceptual types:

```rust
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
    serde::Serialize, serde::Deserialize
)]
pub enum AutonomyLevel {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
    A6,
    A7,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    serde::Serialize, serde::Deserialize
)]
pub enum RiskLevel {
    Low,
    Moderate,
    High,
    Critical,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash,
    serde::Serialize, serde::Deserialize
)]
pub enum Role {
    Orchestrator,
    RiskAssessor,
    RoutingAdvisor,
    BenchmarkAnalyst,
    Researcher,
    RequirementsAnalyst,
    Architect,
    Planner,
    SpecificationWriter,
    Implementer,
    TestPlanner,
    TestExecutor,
    CodeReviewer,
    SecurityReviewer,
    DependencyReviewer,
    UxReviewer,
    AccessibilityReviewer,
    DocumentationWriter,
    IntegrationReviewer,
    ReleasePreparer,
    ReleaseReviewer,
}
```

Policy evaluation:

```rust
pub struct PolicyEvaluationRequest {
    pub project_id: ProjectId,
    pub repository_id: RepositoryId,
    pub work_item_id: Option<WorkItemId>,
    pub role: Option<Role>,
    pub proposed_action: ActionClass,
    pub model_profile_id: Option<ModelProfileId>,
}

pub struct PolicyDecision {
    pub allowed: bool,
    pub effective_autonomy: AutonomyLevel,
    pub required_approvals: Vec<ApprovalKind>,
    pub reasons: Vec<DecisionReason>,
    pub policy_snapshot_id: PolicySnapshotId,
}
```

Routing:

```rust
pub struct RoutingRequest {
    pub work_item_id: WorkItemId,
    pub role: Role,
    pub risk: RiskLevel,
    pub required_capabilities: CapabilityRequirements,
    pub required_modalities: Vec<Modality>,
    pub required_tools: Vec<ToolCapability>,
    pub context_estimate: ContextEstimate,
    pub data_classification: DataClassification,
    pub budget: BudgetConstraint,
    pub excluded_model_fingerprints: Vec<ModelFingerprint>,
}
```

These definitions are illustrative. Implementation may adapt naming to the current codebase while preserving semantics.

---

# 66. Rust Module Architecture

The implementation should use clear internal boundaries.

Recommended modules or workspace crates:

```text
autospec-core
autospec-autonomy
autospec-policy
autospec-risk
autospec-roles
autospec-capabilities
autospec-benchmarks
autospec-routing
autospec-providers
autospec-orchestrator
autospec-workflow
autospec-evidence
autospec-approvals
autospec-budget
autospec-audit
autospec-github
autospec-ux
autospec-documentation
autospec-release
autospec-observability
autospec-storage
autospec-api
autospec-cli
```

The first implementation may begin as modules in the existing crate rather than immediately creating many small crates.

Extraction into crates should occur where it creates real isolation or independent testing value.

## 66.1 Dependency direction

Recommended direction:

```text
core
 ├── autonomy
 ├── policy
 ├── risk
 ├── roles
 └── capabilities

storage
providers
benchmarks
routing
workflow
evidence
approvals
budget
audit

orchestrator
 ├── routing
 ├── workflow
 ├── policy
 ├── evidence
 ├── approvals
 ├── budget
 └── audit

api / cli / github / ux / documentation / release
 └── orchestrator and service interfaces
```

Provider adapters shall not contain policy logic.

UI code shall not directly mutate workflow state without using the orchestrator or service layer.

---

# 67. Provider Interfaces

AutoSpec shall expose a common provider abstraction.

Conceptual interface:

```rust
pub trait ModelProvider: Send + Sync {
    async fn health(&self) -> Result<ProviderHealth, ProviderError>;

    async fn list_models(
        &self,
    ) -> Result<Vec<ModelDescriptor>, ProviderError>;

    async fn estimate(
        &self,
        request: &ExecutionRequest,
    ) -> Result<ExecutionEstimate, ProviderError>;

    async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, ProviderError>;

    async fn cancel(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<(), ProviderError>;
}
```

Adapters may include:

- OpenAI or Codex;
- Anthropic or Claude;
- local llama.cpp;
- OpenAI-compatible local servers;
- command-line agents;
- and future providers.

Provider-specific errors shall be normalized into typed failure categories.

---

# 68. Configuration Schema

Recommended initial configuration:

```yaml
version: 1

autonomy:
  project_level: A5
  operating_mode: enforced

  allow_auto_issue_creation: true
  allow_auto_implementation: true
  allow_auto_pull_requests: true

  allow_auto_merge: false
  allow_auto_release: false
  allow_production_actions: false

risk:
  require_human_approval:
    - critical

  high_risk_max_autonomy: A2
  critical_risk_max_autonomy: A1

separation_of_duties:
  planner_may_review: true
  implementer_may_review: false
  implementer_may_security_review: false
  high_risk_require_provider_diversity: true

routing:
  prefer_qualified_local_models: true
  health_ttl_seconds: 30

  weights:
    predicted_success: 0.45
    historical_acceptance: 0.20
    cost: 0.15
    latency: 0.10
    privacy: 0.10

qualification:
  maximum_age_days: 90
  allow_stale_for_low_risk: false

retry_limits:
  structured_output_repair: 2
  implementation_review_cycles: 3
  test_fix_cycles: 3
  architecture_replans: 2
  provider_failover_attempts: 3

budgets:
  monthly_usd:
    soft: 250
    hard: 350

  per_work_item_usd:
    soft: 10
    hard: 20

github:
  enabled: true
  require_project_board: true
  auto_create_issues: true
  auto_create_pull_requests: true
  auto_merge: false

approvals:
  required_for:
    - destructive_database_change
    - authentication_change
    - authorization_change
    - production_infrastructure
    - public_api_breaking_change
    - governance_change

privacy:
  default_data_classification: internal
  restricted_data_allowed_providers:
    - local-llama-cpp
```

Configuration parsing shall fail clearly for invalid or contradictory settings.

---

# 69. Emergency Controls

AutoSpec shall provide:

- project pause;
- repository pause;
- provider disable;
- work-item cancellation;
- role disable;
- global emergency stop;
- and autonomy reduction.

Emergency stop shall:

1. prevent new executions;
2. request cancellation of active provider operations;
3. prevent new repository mutations;
4. preserve workflow state;
5. record an audit event;
6. and require an authorized resume action.

A model shall never be able to disable or bypass the emergency stop.

---

# 70. Threat Model

The implementation shall explicitly test and mitigate:

## 70.1 Prompt injection from repository content

Repository files may contain instructions attempting to override system policy.

Mitigation:

- policy instructions remain out-of-band;
- repository content is labeled untrusted;
- model output cannot grant authority;
- tool actions are independently authorized.

## 70.2 Malicious or compromised model output

Mitigation:

- structured schemas;
- validation;
- tool permission enforcement;
- independent review;
- sandboxing;
- and evidence gates.

## 70.3 Self-review and collusion

Mitigation:

- fingerprint-based separation of duties;
- provider diversity for high risk;
- independent evidence;
- human escalation.

## 70.4 Secret exfiltration

Mitigation:

- scoped secret injection;
- provider data policy;
- redaction;
- network restrictions;
- audited access.

## 70.5 Infinite autonomous loops

Mitigation:

- retry limits;
- budget limits;
- state-change detection;
- escalation.

## 70.6 Governance modification

Mitigation:

- critical-risk classification;
- human approval;
- protected policy store;
- independent review;
- signed or controlled configuration changes.

## 70.7 Supply-chain compromise

Mitigation:

- dependency review;
- lockfile review;
- vulnerability scanning;
- provenance and artifact hashes.

## 70.8 Audit tampering

Mitigation:

- append-only storage;
- restricted write path;
- optional hash chaining;
- backup and export.

---

# 71. Backward Compatibility

Existing AutoSpec workflows shall continue to function during migration.

The initial default shall not unexpectedly grant higher autonomy.

Recommended default after migration:

```yaml
autonomy:
  project_level: A2
  operating_mode: advisory
```

Existing model configuration shall be imported into the provider registry.

Existing models may receive temporary unverified capability records, but such records shall be visibly marked:

```text
UNVERIFIED
```

Unverified records shall not qualify a model for high-risk autonomous roles.

Existing specifications and issues should be linked into the new work-item model where feasible.

---

# 72. Migration Strategy

Migration shall occur in controlled stages.

## 72.1 Inventory

Identify current:

- provider integrations;
- model configuration;
- benchmark code;
- issue workflow;
- GitHub integration;
- persistence;
- task state;
- and existing orchestration logic.

## 72.2 Compatibility adapters

Wrap existing provider and execution paths behind the new interfaces before replacing behavior.

## 72.3 Shadow operation

Run risk, autonomy, and routing decisions without enforcement.

Compare shadow decisions to current assignments.

## 72.4 Qualification bootstrap

Run benchmark suites against configured models.

Do not rely permanently on manually seeded scores.

## 72.5 Gradual enforcement

Recommended sequence:

1. audit logging;
2. policy decisions;
3. separation of duties;
4. provider health;
5. quota fallback;
6. role qualification;
7. evidence gates;
8. GitHub synchronization;
9. A5 operation;
10. A6 operation;
11. A7 operation.

---

# 73. Implementation Phases

## Phase 0 — Current-state architecture audit

Deliverables:

- map existing modules;
- map provider paths;
- map benchmark paths;
- identify persistence;
- identify GitHub capabilities;
- identify reusable components;
- create ADR for integration strategy.

Exit condition:

- approved migration map;
- no duplicate subsystem implementation started prematurely.

---

## Phase 1 — Core domain and persistence

Implement:

- autonomy levels;
- risk levels;
- roles;
- model fingerprints;
- work items;
- policy snapshots;
- audit events;
- database migrations;
- typed repositories.

Exit condition:

- core types persist and reload;
- migrations pass;
- restart recovery tests pass.

---

## Phase 2 — Policy and risk engine

Implement:

- policy hierarchy;
- most-restrictive evaluation;
- explicit deny;
- effective-autonomy calculation;
- risk classification;
- data and action classification;
- approval requirements.

Exit condition:

- deterministic policy tests pass;
- every policy decision produces an explanation.

---

## Phase 3 — Capability registry and benchmarks

Implement:

- capability taxonomy;
- model profiles;
- benchmark records;
- qualification states;
- model-drift handling;
- local hardware profiles;
- role qualification.

Exit condition:

- at least one hosted and one local profile can be benchmarked and qualified;
- stale qualification handling passes tests.

---

## Phase 4 — Adaptive routing and provider health

Implement:

- provider abstraction;
- health polling;
- capacity and quota status;
- eligible-candidate filtering;
- optimization;
- fallback;
- route explanation;
- local-model preference.

Exit condition:

- exhausted preferred-provider test falls back correctly;
- no-qualified-model test escalates correctly.

---

## Phase 5 — Organizational roles and workflow engine

Implement:

- structured role contracts;
- state machine;
- orchestration;
- role assignment;
- separation-of-duties enforcement;
- retry limits;
- resumability;
- evidence records.

Exit condition:

- complete A3 workflow runs through planning, implementation, testing, review, and documentation;
- self-review is rejected.

---

## Phase 6 — GitHub and mandatory Kanban

Implement:

- issue synchronization;
- dependency links;
- draft PRs;
- project-board creation;
- project fields;
- workflow-status synchronization;
- reconciliation.

Exit condition:

- A5 project can generate and maintain a complete GitHub Project board;
- repeated reconciliation is idempotent.

---

## Phase 7 — UX, documentation, and assurance roles

Implement:

- browser automation adapter;
- screenshot evidence;
- UX review;
- accessibility checks;
- documentation impact detection;
- security-review integration;
- architecture conformance.

Exit condition:

- UI work cannot complete without required visual evidence;
- user-facing changes produce documentation disposition.

---

## Phase 8 — Goal-driven A6 operation

Implement:

- goal intake;
- architecture alternatives;
- plan generation;
- dependency DAG;
- milestones;
- cost estimate;
- approval flow;
- automatic issue creation.

Exit condition:

- a high-level goal can become an approved, executable project plan and GitHub issue hierarchy.

---

## Phase 9 — Controlled A7 operation

Implement:

- autonomous discovery;
- proposal creation;
- policy-controlled promotion;
- technical-debt backlog;
- regression-driven work;
- self-evaluation feedback.

Exit condition:

- AutoSpec can discover and propose work without escalating its own authority;
- governance changes remain blocked pending human approval.

---

## Phase 10 — Hardening and release

Implement:

- emergency stop;
- security testing;
- fault injection;
- restart recovery;
- budget enforcement;
- audit export;
- performance tests;
- migration guide;
- operator documentation.

Exit condition:

- all acceptance criteria pass;
- default configuration remains safe;
- release candidate is independently reviewed.

---

# 74. Test Strategy

## 74.1 Unit tests

Required for:

- autonomy ordering;
- policy merging;
- risk rules;
- role qualification;
- candidate filtering;
- routing scores;
- retry counters;
- workflow transitions;
- approval validity;
- budget calculations;
- and audit serialization.

## 74.2 Property-based tests

Recommended for:

- most-restrictive policy behavior;
- autonomy never increasing through additional restrictions;
- DAG cycle rejection;
- deterministic routing tie-breaks;
- and idempotent reconciliation.

## 74.3 Integration tests

Required for:

- database persistence;
- provider adapters;
- Git worktrees;
- GitHub synchronization;
- benchmark ingestion;
- browser evidence;
- and restart recovery.

## 74.4 End-to-end tests

Required scenarios:

1. A2 implementation workflow.
2. A3 independent-review workflow.
3. A4 provider fallback.
4. A5 GitHub Project workflow.
5. A6 goal decomposition.
6. A7 proposal discovery.
7. high-risk human approval.
8. emergency stop.
9. budget exhaustion.
10. provider outage.
11. invalid structured model output.
12. failed review and bounded retry.

## 74.5 Security tests

Required for:

- prompt injection;
- unauthorized tool request;
- secret leakage;
- protected-branch mutation;
- policy modification;
- audit mutation;
- and governance self-escalation.

## 74.6 Fault injection

Simulate:

- provider timeout;
- partial response;
- malformed response;
- database interruption;
- process crash;
- Git conflict;
- GitHub API failure;
- browser crash;
- and disk-full conditions.

---

# 75. Acceptance Criteria

## AC-001 — Autonomy representation

The system persists and displays A0 through A7.

## AC-002 — Effective autonomy

A work item’s effective autonomy is calculated from all applicable ceilings and never exceeds the lowest applicable ceiling.

## AC-003 — Explainable policy

Every autonomy and permission decision contains machine-readable reasons.

## AC-004 — Risk classification

Every executable work item has a recorded risk classification.

## AC-005 — Role qualification

A model cannot be assigned to a role for which it is not qualified.

## AC-006 — Purpose-specific scores

A high coding score alone does not qualify a model as architect or reviewer.

## AC-007 — Separation of duties

A model fingerprint used for implementation is rejected when proposed as final reviewer for the same work item.

## AC-008 — Planner and reviewer combination

A qualified model may plan and review the same work item only when it did not implement it and policy allows the combination.

## AC-009 — Provider health

A provider-health check occurs before assignment according to configured freshness rules.

## AC-010 — Usage-aware fallback

When the preferred qualified provider has exhausted capacity, AutoSpec selects another qualified provider.

## AC-011 — Safe failure

When no qualified model is available, AutoSpec escalates rather than assigning an unqualified model.

## AC-012 — Local models

A qualified local model can participate in routing and execution.

## AC-013 — Runtime-specific qualification

Different quantizations or material runtime configurations are represented as distinct profiles.

## AC-014 — Stale qualification

A stale model qualification is flagged and blocked according to policy.

## AC-015 — Structured output

Invalid role output is rejected and repaired or escalated within retry limits.

## AC-016 — Workflow enforcement

Mandatory workflow stages cannot be skipped without an authorized waiver.

## AC-017 — Evidence gates

A stage cannot pass based solely on an agent claim when deterministic evidence is required.

## AC-018 — Independent test planning

At A3 and above, test planning is independent from implementation.

## AC-019 — UX validation

A UI-affecting work item configured to require UX review cannot complete without rendered visual evidence.

## AC-020 — Documentation disposition

A user-facing change records either documentation updates or a justified no-documentation decision.

## AC-021 — Security review

Security-sensitive changes automatically require a security-review role.

## AC-022 — Retry limits

Repeated implementation or review failures stop at the configured retry limit and escalate.

## AC-023 — Restart recovery

An interrupted workflow can be safely reconstructed after process restart.

## AC-024 — Idempotent GitHub reconciliation

Repeated GitHub Project reconciliation does not create duplicate issues, fields, or board entries.

## AC-025 — Mandatory A5 board

An A5 project cannot become operational without a linked and valid GitHub Project board unless an explicitly supported alternative platform is configured.

## AC-026 — Goal decomposition

An A6 goal produces a dependency-safe work-item DAG with acceptance criteria and estimated budget.

## AC-027 — Autonomous discovery boundaries

An A7 project may propose work but cannot raise its own autonomy or remove approval requirements.

## AC-028 — Human approval

A high-risk or critical action requiring approval remains blocked until the correct approval exists.

## AC-029 — Revision-bound approval

A material specification revision invalidates approval where policy requires renewed review.

## AC-030 — Budget enforcement

A hard budget prevents additional billable execution without authorized override.

## AC-031 — Budget safety

Budget exhaustion does not cause routing to an unqualified model.

## AC-032 — Audit completeness

Every role assignment, workflow transition, approval, override, and repository mutation produces an audit event.

## AC-033 — Explainable routing

The UI or CLI shows why the selected model won and why rejected candidates were excluded.

## AC-034 — Emergency stop

Emergency stop prevents new work and attempts to cancel active executions.

## AC-035 — Protected branch safety

Agents cannot directly push to protected branches under the default policy.

## AC-036 — Secret protection

Secret values are not written into ordinary prompts, model transcripts, or logs unless explicitly authorized.

## AC-037 — Prompt-injection resistance

Repository instructions cannot override organization or project policy.

## AC-038 — Provider diversity

High-risk review enforces provider or model-family diversity when configured.

## AC-039 — Traceability

A completed requirement links to implementation, tests, review, and pull request evidence.

## AC-040 — Backward compatibility

Existing AutoSpec A1/A2 workflows continue to function during migration.

## AC-041 — Safe default

New installations do not default to automatic merge, release, or production authority.

## AC-042 — Metrics

The system records cost, retries, acceptance, and escalation metrics by model and role.

## AC-043 — Model drift

A provider or model-version change triggers requalification according to policy.

## AC-044 — Concurrency safety

Conflicting work items are isolated, blocked, or reconciled before merge.

## AC-045 — Governance protection

Changes to autonomy, policy enforcement, audit handling, or emergency controls are classified as critical risk.

---

# 76. Definition of Done

This specification is complete only when:

- all mandatory phases are implemented;
- all acceptance criteria pass;
- database migrations are reversible or have documented recovery;
- policy and routing behavior is covered by automated tests;
- at least one local and one hosted provider are supported;
- role-specific benchmark qualification is operational;
- provider-capacity fallback is operational;
- separation of duties is enforced;
- GitHub Projects and Kanban synchronization are operational;
- UX validation is integrated for UI work;
- documentation review is integrated;
- audit and explainability interfaces are available;
- emergency controls are tested;
- migration documentation exists;
- operator documentation exists;
- configuration reference exists;
- security review is complete;
- and an independent model or human reviewer verifies the implementation against this specification.

---

# 77. End-to-End Example

Human request:

> Add organization-level SSO support to AutoSpec.

## Step 1 — Goal intake

AutoSpec records the goal and classifies it as involving authentication and authorization.

Initial risk:

```text
HIGH
```

Project autonomy:

```text
A6
```

Risk policy maximum:

```text
A2
```

Effective autonomy:

```text
A2
```

Reason:

```text
Authentication boundary requires architecture and security approval.
```

## Step 2 — Planning

The router selects a qualified architect.

A local coding model is rejected for architecture because its textual-analysis and architecture scores are below the required thresholds.

The architect produces:

- protocol options;
- identity-provider assumptions;
- threat analysis;
- data model;
- migration strategy;
- work-item DAG;
- acceptance criteria;
- and rollback plan.

## Step 3 — Approval

AutoSpec creates:

- architecture approval;
- security approval;
- and specification approval requests.

No implementation starts until approvals are recorded.

## Step 4 — Work decomposition

GitHub issues are created:

```text
Epic: Organization-level SSO

├── Define identity-provider abstraction
├── Add organization authentication configuration
├── Implement OIDC provider
├── Implement SAML provider
├── Add session and token validation
├── Add authorization policy integration
├── Add migration path
├── Add security tests
├── Add browser UX flow
├── Add operator documentation
└── Add release and rollback plan
```

Dependencies are added to the GitHub Project.

## Step 5 — Model assignments

```text
Planner: qualified high-reasoning model
Implementer: qualified Rust coding model
Test planner: independent reasoning model
Security reviewer: qualified security model
Code reviewer: qualified independent reviewer
UX reviewer: vision-capable model
Documentation writer: qualified documentation model
```

The implementation model is excluded from final review.

## Step 6 — Implementation

Each work item runs in an isolated worktree.

The implementer modifies only approved scope.

Build, lint, and unit tests execute.

Evidence is stored.

## Step 7 — Assurance

The independent test plan executes:

- unit tests;
- integration tests;
- authentication failure cases;
- invalid-token cases;
- privilege-escalation cases;
- browser sign-in flows;
- and migration rehearsal.

The security reviewer identifies one major issue involving incomplete issuer validation.

The workflow returns to implementation.

## Step 8 — Bounded correction

A second implementation attempt fixes issuer validation.

Tests pass.

Security review passes.

UX review validates:

- sign-in;
- error states;
- organization selection;
- logout;
- and expired-session behavior.

## Step 9 — Documentation and integration

AutoSpec updates:

- configuration reference;
- deployment guide;
- security documentation;
- migration guide;
- troubleshooting;
- and changelog.

The integration reviewer confirms requirement traceability.

## Step 10 — Pull request

A draft pull request becomes ready for human merge approval.

The PR contains links to:

- approved specification;
- architecture decision;
- test evidence;
- security review;
- screenshots;
- documentation changes;
- and rollback plan.

AutoSpec does not merge automatically because project policy disallows automatic merging of authentication changes.

---

# 78. Recommended Epic and Issue Breakdown

## Epic 1 — Core autonomy domain

- Add autonomy-level types.
- Add risk-level types.
- Add policy scopes.
- Add effective-autonomy evaluator.
- Add policy snapshots.
- Add audit-event domain types.

## Epic 2 — Persistence foundation

- Add schema migrations.
- Add repositories for policies and work items.
- Add execution-attempt persistence.
- Add evidence persistence.
- Add approval persistence.
- Add restart reconstruction.

## Epic 3 — Model and capability registry

- Add provider records.
- Add model fingerprints.
- Add runtime profiles.
- Add capability taxonomy.
- Add role qualification.
- Add qualification-staleness rules.

## Epic 4 — Benchmark integration

- Add task-specific benchmark suites.
- Add local-hardware metadata.
- Add benchmark result ingestion.
- Add qualification calculation.
- Add benchmark dashboard.

## Epic 5 — Adaptive routing

- Add provider-health interface.
- Add quota and capacity checks.
- Add candidate filtering.
- Add routing utility calculation.
- Add fallback.
- Add route explanation.

## Epic 6 — Role and workflow engine

- Add role contracts.
- Add workflow states.
- Add transition guards.
- Add role assignment.
- Add separation-of-duties enforcement.
- Add retries and escalation.

## Epic 7 — Evidence and assurance

- Add command evidence.
- Add test evidence.
- Add code-review contracts.
- Add security-review contracts.
- Add architecture conformance.
- Add requirement traceability.

## Epic 8 — GitHub and Kanban

- Add issue synchronization.
- Add dependency synchronization.
- Add project-board creation.
- Add project fields.
- Add workflow reconciliation.
- Add draft-PR flow.

## Epic 9 — UX and documentation

- Add browser automation adapter.
- Add screenshot evidence.
- Add visual-review role.
- Add accessibility review.
- Add documentation-impact analysis.
- Add documentation gate.

## Epic 10 — Goal-driven development

- Add goal intake.
- Add goal-plan schema.
- Add architecture alternatives.
- Add work-item DAG generation.
- Add milestone generation.
- Add approval flow.

## Epic 11 — Autonomous discovery

- Add candidate-work detectors.
- Add proposal state.
- Add policy-controlled promotion.
- Add technical-debt backlog.
- Add regression-driven discovery.

## Epic 12 — Operations and governance

- Add budgets.
- Add emergency stop.
- Add dashboards.
- Add audit export.
- Add release management.
- Add monitoring adapters.
- Add rollback preparation.

---

# 79. Required Implementation Order

The implementation shall proceed in this dependency order:

```text
Core domain
  → persistence
  → policy and risk
  → model registry
  → benchmark qualification
  → provider health
  → routing
  → role contracts
  → workflow engine
  → evidence and approvals
  → separation of duties
  → GitHub and Kanban
  → UX and documentation
  → A6 goal decomposition
  → A7 autonomous discovery
  → hardening and release
```

A6 and A7 work shall not begin before:

- policy enforcement;
- audit;
- qualification;
- separation of duties;
- bounded retries;
- and emergency controls

are functioning.

---

# 80. Final Product Definition

When this specification is complete, AutoSpec shall no longer be described merely as a specification generator or multi-agent coding tool.

It shall be accurately described as:

> **A governed orchestration and assurance platform for progressing software development from AI-assisted work to an adaptive, autonomous software-engineering organization.**

The defining capabilities shall be:

- purpose-qualified models;
- adaptive provider routing;
- independent planning, implementation, and review;
- explicit autonomy boundaries;
- evidence-based validation;
- mandatory project visibility;
- bounded execution;
- human-controlled authority;
- and continuous organizational learning.

This document supersedes the previously truncated autonomy specification.