# AutoSpec Specification: Repository-Derived Real-Work Benchmarking and Model Qualification

**Date:** 2026-08-16  
**Status:** Implementation-ready — Phases 1–2 decomposable; Phases 3 and 7 gated (see below)  
**Parent:** `docs/specs/2026-08-16-multi-model-engineering-team-design.md`  
**Related:**
- `docs/specs/2026-08-16-benchmark-per-evaluation-telemetry-design.md`
- `docs/specs/2026-08-16-vision-image-generation-qualification-design.md`
- `docs/specs/2026-08-16-autonomous-engineering-organization-design.md` (AS-AEO-001)
- `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md`

## Authority over the benchmark subsystem

Four documents describe benchmarking. **This one is authoritative.** They are
complementary layers, not competing designs:

| Document | Layer |
|---|---|
| **This spec** | Corpus, historical-replay methodology, difficulty and scoring model, qualification rules, `autospec bench` CLI (§49), `crates/autospec-bench/` (§51) |
| `2026-08-16-benchmark-per-evaluation-telemetry-design.md` | Deepens §31 — the per-evaluation metric contract, token-weighted throughput, context/performance curves |
| `2026-08-16-vision-image-generation-qualification-design.md` | A task family alongside the RealWork families — vision/image-generation suites, accuracy metrics, candidate matrix |
| AS-AEO-001 Epic 4 | Integration point only. Rescoped to *ingest* this system's results, not to define a second one |

Where the amendments and this spec describe the same CLI verb or metric, this
spec's §31, §49 and §53 are canonical.

## Consistency with decisions already recorded

Checked against `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md`;
no conflict, and three points align deliberately:

- **§53 result ledger** — "SHALL append to the existing benchmark/routing ledger."
  Consistent with **D3**: the append-only JSONL ledger is the system of record and
  any database is a projection. Do not introduce a separate benchmark store.
- **§31 telemetry** — "SHALL remain `unknown` when unavailable … MUST NOT fabricate."
  Same rule already binding on #3174.
- **§51 Rust** — `crates/autospec-bench/` is consistent with **D5** and AS-AEO-001
  §66. Per §66.1, the benchmark crate sits beside the domain layer; it must not
  become a second home for routing policy.

## Decomposition gates

- **Phases 1–2 (corpus framework, public seed corpus) are decomposable now.** They
  depend on no open, parked, or unbuilt subsystem, and `crates/autospec-cli`'s
  `commands/benchmark.rs` is currently a three-line `not_implemented` stub.
- **Phase 3 (private Tier-1 corpus) is gated on §36.** Mining LC-BinBase Scheduler,
  the WCMC applications, and private Go modules means processing real production
  repositories that contain credentials, customer data, and protected datasets. The
  §36 detection-and-removal path and the §37 access-level model MUST exist, be
  tested against known-positive fixtures, and be independently reviewed **before**
  any private repository is mined. This is a hard gate, not a sequencing preference.
- **Phase 7 (router integration) is gated on the router existing.** The deterministic
  router was parked by D1 (#3170/#3171) and is rebuilt as AS-AEO-001 Epic 5. §46
  cannot be implemented before then.
- **Phases 4–6** may be decomposed once Phase 1 lands.

---

## 1. Executive Summary

AutoSpec must evaluate AI models against the **actual engineering work performed across our repositories**, rather than relying primarily on synthetic coding benchmarks such as HumanEval, MBPP, generic agent benchmarks, or isolated algorithm problems.

The benchmark system SHALL therefore introduce a repository-derived benchmark family called:

```text
realwork-v1
```

`realwork-v1` SHALL be constructed from real historical engineering tasks found in:

- LC-BinBase Scheduler
- major WCMC production applications
- shared Go modules and libraries
- WCMC infrastructure and deployment repositories
- MoNA
- CTS-Lite
- MassCube
- AutoSpec
- other representative repositories as appropriate

The most important benchmark material SHALL come from **long-lived, multi-year production systems**, particularly LC-BinBase Scheduler and major WCMC applications.

These systems test engineering abilities that synthetic coding benchmarks do not adequately measure:

- understanding legacy architecture;
- reconstructing historical design intent;
- safely modifying long-lived systems;
- debugging cross-module and cross-repository behavior;
- maintaining backward compatibility;
- reasoning about databases, queues, scheduling and distributed state;
- handling scientific data and numerical correctness;
- making narrowly scoped production fixes;
- designing appropriate tests;
- reviewing changes made by another model;
- identifying where a problem actually resides;
- avoiding unnecessary modernization or rewrites;
- understanding deployment and infrastructure consequences.

The benchmark SHALL evaluate models by **engineering role and task purpose**.

A model SHALL NOT receive one universal "coding score."

Instead AutoSpec SHALL maintain capability profiles such as:

```text
implementation.rust
implementation.go
implementation.java_scala
implementation.python_scientific
implementation.shell

diagnosis.distributed_systems
diagnosis.legacy_application
diagnosis.cross_repository

planning.architecture
planning.feature
planning.migration

review.security
review.correctness
review.legacy_change
review.api_compatibility

testing.unit
testing.integration
testing.e2e
testing.fault_injection

documentation.technical
ux.visual_verification
```

These capability profiles SHALL feed directly into AutoSpec's model router.

---

# 2. Goals

The benchmark system SHALL answer:

> Which model should AutoSpec select for this particular kind of engineering work?

It SHALL determine, with empirical evidence:

1. whether a model can understand the affected system;
2. whether it can identify the actual root cause;
3. whether it chooses an appropriate change;
4. whether it implements the change correctly;
5. whether it preserves existing behavior;
6. whether it writes meaningful tests;
7. whether it can review another implementation;
8. whether it can reason across repositories;
9. whether it handles legacy systems carefully;
10. whether it can work with scientific/numerical code;
11. whether it can work with infrastructure and operational systems;
12. how much time, context, inference and compute it requires;
13. how often it requires repair attempts;
14. whether a cheaper or local model is sufficient for that class of task.

---

# 3. Non-Goals

`realwork-v1` SHALL NOT:

- reward patch similarity to the historical implementation;
- assume the original human solution was the only valid solution;
- expose historical fixes to the evaluated model;
- require wholesale modernization of old applications;
- rank models using only one scalar score;
- treat tokens/second as equivalent to productivity;
- make public private repository contents;
- use private production secrets as benchmark fixtures;
- execute benchmark jobs against live production infrastructure;
- assume LOC or number of files equals difficulty;
- allow known post-fix documentation to leak into an evaluation.

---

# 4. Core Principle: Historical Repository Replay

The fundamental benchmark method SHALL be **historical replay**.

For each benchmark task AutoSpec SHALL identify:

```text
repository
pre-fix commit
historical problem
historical human fix
observable expected behavior
relevant tests
hidden evaluation tests
```

The evaluator SHALL check out the repository at the point immediately before the real fix.

Conceptually:

```text
real production problem
        │
        ▼
historical issue / PR / commit
        │
        ▼
identify pre-fix SHA
        │
        ▼
construct benchmark fixture
        │
        ▼
hide historical solution
        │
        ▼
model independently solves problem
        │
        ▼
behavioral evaluation
```

The historical patch is an **oracle for benchmark authorship**, not an expected diff.

Alternative correct solutions SHALL receive full credit if they satisfy the behavioral and architectural acceptance criteria.

---

# 5. Repository Corpus

## 5.1 Tier 1 — Long-Lived Production Systems

Tier 1 SHALL receive the highest weighting.

Initial Tier 1 sources:

```text
LC-BinBase Scheduler
major WCMC applications
WCMC shared services
long-lived production support applications
```

Recommended benchmark contribution:

**35–40% of scored RealWork evaluations.**

These applications are particularly valuable because they contain:

- years of architectural evolution;
- historical compatibility requirements;
- old and new idioms mixed together;
- production data assumptions;
- scheduling and orchestration;
- long-running processes;
- distributed state;
- operational edge cases;
- real regressions;
- legacy dependencies;
- accumulated technical debt;
- domain-specific scientific logic.

---

# 6. LC-BinBase Scheduler Benchmark Family

A dedicated task family SHALL exist:

```text
realwork/lc-binbase-scheduler
```

It SHOULD contain historical tasks involving:

### Scheduling

- job admission;
- scheduling;
- retries;
- delayed jobs;
- cancellation;
- dependency ordering;
- prioritization;
- resumption after failure;
- duplicate work prevention.

### State

- persisted state;
- stale state;
- state transition bugs;
- state reconciliation;
- interrupted jobs;
- partial completion;
- restart recovery.

### Distributed behavior

- worker failure;
- transient services;
- database outages;
- communication failure;
- duplicate execution;
- concurrency;
- race conditions.

### Scientific pipeline behavior

- input validation;
- sample lifecycle;
- batch processing;
- metadata propagation;
- result persistence;
- partially processed datasets.

### Legacy maintenance

- changing behavior without rewriting unrelated components;
- compatibility with older components;
- old library versions;
- mixed architectural styles;
- undocumented assumptions.

These tasks SHALL intentionally evaluate whether a model understands an old system before changing it.

---

# 7. Longitudinal Replay

Large applications SHALL NOT only be benchmarked at their current state.

AutoSpec SHALL support multiple historical generations.

For example:

```text
lc-binbase-scheduler/
    2017/
    2019/
    2021/
    2023/
    2025/
    current/
```

These need not align exactly to calendar years. A generation MAY instead correspond to meaningful architectural eras.

Each generation SHALL have its own:

```text
commit SHA
dependency graph
build environment
supported runtime
toolchain
architecture snapshot
known historical issues
```

This allows models to be evaluated against:

- older Java;
- older Scala;
- older Go;
- old framework versions;
- previous database systems;
- older CI/deployment conventions.

AutoSpec SHALL penalize models that blindly attempt to convert historical applications to current idioms when the requested fix does not require it.

---

# 8. Tier 2 — Shared Go Modules

Shared Go code SHALL form a dedicated benchmark domain:

```text
realwork/go-library-maintenance
```

Recommended benchmark contribution:

**15–20%.**

This family SHALL test more than basic Go syntax.

Examples SHALL include:

### API evolution

Change a shared API while preserving valid existing consumers.

### Compatibility

Determine whether callers require modification.

### Dependency propagation

A model changes library `A`.

AutoSpec then expects it to reason about:

```text
module A
   ├── app B
   ├── service C
   └── tool D
```

The model MUST determine which dependents actually require changes.

### Version management

Tests SHOULD cover:

- `go.mod`;
- module replacements;
- semantic versions;
- dependency updates;
- transitive changes.

### Concurrency

Historical cases involving:

- goroutines;
- channels;
- cancellation;
- contexts;
- synchronization;
- leaks.

### Error handling

- wrapped errors;
- sentinel errors;
- API error contracts;
- failure propagation.

---

# 9. Cross-Repository Engineering

AutoSpec SHALL introduce a first-class evaluation class:

```text
cross_repository_diagnosis
```

This is a major requirement.

Real engineering incidents frequently cross repository boundaries.

Example:

```text
Production failure
      │
      ├── application
      │
      ├── shared Go module
      │
      ├── database
      │
      ├── Docker definition
      │
      └── deployment configuration
```

The model SHOULD initially receive the symptom rather than the solution location.

It must determine where the problem originates.

Scoring SHALL include:

```text
root-cause repository selection
subsystem selection
dependency reconstruction
relevant file discovery
irrelevant repository avoidance
minimal fix selection
consumer impact analysis
deployment impact analysis
```

This capability SHALL influence AutoSpec's **diagnoser** and **planner** model routing.

---

# 10. WCMC Infrastructure Benchmark Family

A benchmark family SHALL cover production infrastructure:

```text
realwork/wcmc-infrastructure
```

Examples include:

- Ansible;
- Docker;
- Docker Swarm;
- KVM;
- NFS;
- Linux;
- storage;
- networking;
- host inventories;
- environment-specific configuration;
- production/test/dev separation;
- databases;
- logging;
- monitoring;
- messaging.

The evaluator SHALL distinguish between:

```text
application defect
deployment defect
configuration defect
infrastructure defect
```

Models SHALL receive extra credit for correctly proving that **no application-code modification is needed** where appropriate.

---

# 11. Scientific Computing

Scientific workloads SHALL have a distinct family:

```text
realwork/scientific-computing
```

Sources include:

- LC-BinBase;
- MassCube;
- MoNA;
- other metabolomics tooling.

Evaluation MUST NOT rely only on exact output equality when floating-point behavior permits tolerances.

Task metadata SHALL support:

```text
absolute_tolerance
relative_tolerance
allowed_numeric_variance
expected_statistical_property
performance_constraint
```

Tasks SHOULD include:

- LC-MS processing;
- peak detection;
- feature alignment;
- annotation;
- chemical identifiers;
- mass spectra;
- data imports;
- numerical algorithms;
- performance optimization;
- parallel processing;
- scientific data validation.

---

# 12. Modern Application Engineering

MoNA and CTS-Lite SHALL continue to provide modern full-stack examples.

These SHALL test:

```text
backend
frontend
API
database
data pipelines
E2E
accessibility
observability
resilience
```

Examples already identified include:

### CTS-Lite

```text
scientific dataset
    ↓
Go model
    ↓
API
    ↓
browser UI
    ↓
Playwright
```

### MoNA

Examples include:

- RabbitMQ recovery;
- persistence bugs;
- indexing;
- null-handling;
- front-end chart lifecycle;
- accessibility;
- chemical validation.

---

# 13. AutoSpec as a Benchmark Source

AutoSpec itself SHALL remain a benchmark source.

Recommended contribution:

**15–20%.**

It is particularly useful for:

- Rust;
- shell;
- Bats;
- CI;
- process supervision;
- concurrency;
- security;
- GitHub API behavior;
- architecture analysis;
- autonomous-agent behavior;
- prompt/context optimization.

AutoSpec tasks SHALL NOT dominate the benchmark because the benchmark exists to evaluate models for the wider engineering environment.

---

# 14. Recommended Corpus Distribution

Initial target:

| Domain | Weight |
|---|---:|
| LC-BinBase Scheduler + large WCMC applications | 35% |
| Shared Go modules | 15% |
| MoNA / CTS-Lite / application engineering | 15% |
| Scientific computing | 10% |
| AutoSpec | 15% |
| Infrastructure / operational engineering | 10% |

Cross-repository scenarios MAY span multiple categories and SHALL be tracked independently.

Weighting MUST be configurable.

---

# 15. Engineering Roles

Every suitable benchmark problem SHOULD generate several role variants.

## 15.1 Diagnoser

Input:

```text
symptom
logs
repository access
test results
```

Expected output:

```text
root cause
affected subsystem
evidence
recommended remediation
```

The diagnoser SHALL NOT necessarily modify code.

---

# 16. Implementer

The implementer receives:

```text
problem statement
repository checkout
normal engineering tools
```

and must implement a fix.

---

# 17. Test Engineer

The test engineer must identify how the issue SHOULD be tested.

This role is especially useful for:

- race conditions;
- restarts;
- malformed input;
- message loss;
- fault recovery;
- retries;
- old regressions.

The evaluation SHALL score not just number of tests, but whether they reproduce the actual failure mode.

---

# 18. Reviewer

Reviewer variants SHALL receive a candidate patch.

Some candidate patches SHOULD deliberately contain subtle flaws.

Example:

```text
Patch fixes visible problem
BUT
introduces duplicate job execution during restart
```

A reviewer succeeds only if it identifies the hidden regression.

---

# 19. Planner / Architect

Planning benchmarks SHALL test whether the model:

- understands current architecture;
- traces dependencies;
- identifies affected systems;
- avoids imaginary components;
- sequences implementation safely;
- proposes migration only where necessary.

AutoSpec SHALL especially use these scores when choosing planning models.

---

# 20. Documentation

Documentation benchmarks MAY test:

- operator documentation;
- developer documentation;
- migration notes;
- runbooks;
- release notes;
- architecture descriptions.

Correctness SHALL outweigh prose style.

---

# 21. UI/UX Verification

Where repository tasks include user-facing behavior, benchmark variants MAY include visual verification using browser automation and vision-capable models.

This SHALL integrate with the vision-model qualification amendment.

---

# 22. Difficulty Classification

Difficulty SHALL NOT be derived from LOC.

Use:

```text
S
M
L
XL
```

### S — Surgical

Example:

```text
one local defect
small subsystem
known reproduction
```

### M — Subsystem

Requires understanding several files and interactions.

### L — System

Requires reasoning across layers.

Examples:

```text
API + DB + UI
message bus + persistence
shared library + consumers
```

### XL — Architectural

Examples:

```text
cross-repository root cause
distributed consistency
security boundary
major concurrency behavior
architecture migration
legacy subsystem interaction
```

---

# 23. Historical Task Mining

AutoSpec SHOULD provide automated benchmark-candidate mining.

Proposed command:

```bash
autospec bench mine \
  --repo org/repository \
  --since 2017-01-01
```

The miner SHALL inspect:

- merged PRs;
- closed issues;
- commits;
- commit messages;
- changed files;
- tests;
- labels;
- discussion;
- CI history where available.

It SHOULD identify promising candidates using signals including:

```text
bug
fix
regression
race
performance
failure
timeout
restart
compatibility
migration
panic
crash
null
incorrect
deadlock
recovery
security
```

Candidate discovery MAY use a model.

**Benchmark truth generation SHALL NOT depend solely on model judgment.**

---

# 24. Candidate Qualification Pipeline

```text
GitHub history
    ↓
candidate discovery
    ↓
historical fix inspection
    ↓
pre-fix SHA identification
    ↓
reproduction verification
    ↓
hidden acceptance test generation
    ↓
counterfactual generation
    ↓
benchmark review
    ↓
corpus
```

No benchmark becomes `calibrated` until its fixture is independently reproducible.

---

# 25. Manifest Format

Each benchmark SHALL have a machine-readable manifest.

Example:

```yaml
schema_version: 1

id: RW-LCBS-0042
suite: realwork-v1

source:
  organization: private
  repository: lc-binbase-scheduler
  visibility: private

history:
  base_commit: abcdef123456
  fix_commit: fedcba654321

task:
  family: distributed_state
  role: implementer
  difficulty: L

languages:
  - java
  - scala

capabilities:
  - legacy_reasoning
  - scheduling
  - persistence
  - failure_recovery

prompt:
  file: prompt.md

evaluation:
  baseline:
    command: ./benchmark/baseline.sh

  public_tests:
    command: ./benchmark/public-tests.sh

  hidden_tests:
    command: ./benchmark/hidden-tests.sh

scoring:
  behavioral_correctness: 40
  regression_safety: 20
  scope_discipline: 10
  legacy_restraint: 10
  test_quality: 10
  architecture_compliance: 10

telemetry:
  enabled: true

privacy:
  publish_source: false
  publish_prompt: false
  publish_patch: false
```

---

# 26. Scoring Model

## 26.1 Mandatory dimensions

Each applicable task SHALL score:

### Behavioral correctness

Did the solution work?

### Regression safety

Did existing behavior remain valid?

### Scope discipline

Did the model avoid unrelated changes?

### Test quality

Did it write appropriate tests?

### Architectural compliance

Did it preserve required invariants?

### Root-cause accuracy

For diagnostic tasks.

### Compatibility

For shared modules and legacy systems.

### Security

Where applicable.

---

# 27. Legacy Restraint

A new required metric SHALL be introduced:

```text
legacy_restraint
```

It measures whether a model avoids harmful modernization.

Penalties SHALL include:

- gratuitous framework replacement;
- unrelated refactoring;
- large dependency upgrades;
- API redesign without need;
- rewriting modules instead of fixing the defect;
- introducing new architectural patterns without justification;
- unrelated formatting churn.

This is especially important for LC-BinBase Scheduler and WCMC.

Example result:

| Model | Correctness | Scope | Legacy restraint |
|---|---:|---:|---:|
| Model A | 96 | 94 | 97 |
| Model B | 98 | 71 | 43 |
| Model C | 91 | 99 | 99 |

AutoSpec might therefore prefer Model C for some legacy maintenance tasks despite Model B's higher raw correctness.

---

# 28. Scope Discipline Metrics

AutoSpec SHOULD record:

```text
files inspected
files changed
unnecessary files changed
lines added
lines deleted
dependencies changed
public APIs changed
unrelated formatting changes
```

These SHALL NOT be interpreted mechanically.

Large legitimate changes MUST remain possible.

---

# 29. First-Pass Success

A critical metric SHALL be:

```text
first_pass_success
```

A model that solves a task correctly on the first attempt has meaningful operational advantages.

Record:

```text
attempt_count
repair_turns
failed_test_cycles
human_intervention
```

---

# 30. Cost of Successful Work

The benchmark SHALL report:

```text
cost_per_successful_task
seconds_per_successful_task
input_tokens_per_successful_task
output_tokens_per_successful_task
inference_seconds_per_successful_task
energy_per_successful_task
```

This builds directly on the benchmark telemetry specification.

---

# 31. Required Inference Telemetry

Every model invocation SHALL record, where available:

```text
input tokens
output tokens
prompt processing time
generation time
prompt tok/s
decode tok/s
TTFT
wall-clock time
peak GPU memory
peak host memory
power
energy
model
quantization
backend
hardware
context length
```

Metrics SHALL remain `unknown` when unavailable.

AutoSpec MUST NOT fabricate them.

---

# 32. Throughput Is Not Productivity

Tokens/sec SHALL be prominently reported, but SHALL never determine qualification alone.

Example:

```text
Model A
  82 tok/s
  3 repair cycles
  total task time: 14m

Model B
  37 tok/s
  first-pass success
  total task time: 5m
```

For this task, Model B SHALL be treated as more productive.

---

# 33. Cross-Repository Scoring

Cross-repository evaluation SHALL capture:

```text
candidate repositories inspected
correct repository
correct module
false-positive repositories
dependency graph accuracy
consumer impact accuracy
deployment impact accuracy
```

Example score:

```text
Repository selection         15
Root cause                   25
Dependency reconstruction    15
Implementation               25
Regression safety            10
Scope discipline             10
```

---

# 34. Counterfactual Variants

Historical public tasks may have appeared in model training data.

Therefore every sufficiently important historical task SHOULD produce counterfactual variants.

Counterfactuals preserve the engineering mechanism while altering surface details.

Example original:

```text
worker retries after RabbitMQ disconnect
```

Counterfactual:

```text
different queue
different timeout
different recovery order
different test fixture
same underlying recovery defect
```

Other transformations MAY modify:

- variable names;
- test data;
- ordering;
- timings;
- entity identifiers;
- fixture shape;
- failure sequence;
- affected consumer.

Counterfactuals MUST remain behaviorally valid.

---

# 35. Public and Private Suites

The suite SHALL support:

```text
realwork-public-v1
realwork-private-v1
```

## Public

May use:

- AutoSpec;
- MoNA;
- CTS-Lite;
- MassCube;
- public WCMC repositories.

Useful for reproducibility and community comparisons.

## Private

May use:

- LC-BinBase Scheduler;
- private WCMC applications;
- private Go modules;
- private internal historical work.

Private benchmark source SHALL never be published by AutoSpec.

Reports MAY expose:

```text
task ID
task family
difficulty
score
model
hardware
performance
```

without exposing the source code or prompt.

---

# 36. Secret Handling

Before creating a benchmark from a private repository, AutoSpec MUST detect and remove:

- passwords;
- tokens;
- private URLs;
- customer data;
- credentials;
- certificates;
- private SSH data;
- production identifiers where required;
- protected datasets.

Benchmark fixtures SHALL run in isolated environments.

---

# 37. Repository Access Levels

AutoSpec SHALL distinguish:

```text
public
private-local
private-connected
restricted
```

The benchmark builder SHALL never assume a repository is absent merely because the current GitHub connector cannot see it.

Repository catalogs SHALL be explicitly configurable.

Example:

```toml
[[benchmark.sources]]
repository = "org/lc-binbase-scheduler"
visibility = "private"
priority = "tier1"
```

---

# 38. Environment Reproducibility

Historical software may depend on obsolete environments.

Each task SHALL define its runtime.

Preferred implementation:

```text
OCI container
Nix/devcontainer where necessary
pinned dependencies
```

A task manifest MAY declare:

```yaml
environment:
  image: autospec-bench/lcbs-2021:sha256:...
```

The environment SHALL be immutable after corpus release.

---

# 39. External Service Simulation

Benchmarks MUST NOT require production systems.

Dependencies such as:

- PostgreSQL;
- RabbitMQ;
- Redis;
- Elasticsearch;
- Docker;
- HTTP APIs;
- NFS-like behavior

SHOULD be implemented using:

- local containers;
- fixtures;
- mock services;
- test doubles;
- fault-injection proxies.

---

# 40. Fault Injection

Fault-injection scenarios SHALL be first-class benchmark tasks.

Examples:

```text
kill message broker
restart database
delay API
drop network connection
kill worker
duplicate message
stale cache
partial write
timeout response
```

A model's solution SHALL be evaluated under the actual failure sequence.

---

# 41. Numerical Evaluation

Scientific benchmark tasks SHALL support validation such as:

```text
max absolute error
relative error
distribution similarity
feature count
rank correlation
expected peak locations
mass tolerance
retention-time tolerance
```

Historical numerical outputs MAY be used as references where scientifically valid.

---

# 42. Review Benchmarks

AutoSpec SHALL automatically generate some reviewer tasks from historical failures.

Example:

1. take historical buggy change;
2. present it as a candidate PR;
3. ask the review model to assess it;
4. measure whether it detects the known issue.

Variants MAY contain intentionally flawed model-generated patches.

Reviewer scoring SHOULD prioritize **defect recall with low false-positive noise**.

---

# 43. Test-Planning Benchmarks

Test roles SHALL be scored separately from implementation.

A strong test engineer should identify:

```text
happy path
failure path
regression path
boundary condition
concurrency behavior
recovery
integration behavior
```

This provides evidence for AutoSpec's dedicated test-planning role.

---

# 44. Multi-Agent Benchmarking

AutoSpec SHALL eventually evaluate complete teams.

Example:

```text
Planner
   ↓
Implementer
   ↓
Tester
   ↓
Reviewer
```

The team SHALL obey separation-of-duty rules.

A model SHALL NOT implement and subsequently be the sole reviewer of the same work item.

Team benchmarks SHALL record:

```text
planner model
implementer model
test model
review model
total tokens
total cost
total time
repair cycles
final correctness
```

---

# 45. Capability Calibration

Models SHALL move through capability states:

```text
unknown
observed
candidate
calibrated
degraded
disqualified
```

A model SHALL NOT become `calibrated` for a role merely because it performs well on HumanEval or other synthetic benchmarks.

At least one relevant RealWork family SHALL be required.

For important production roles, multiple task families SHOULD be required.

---

# 46. Routing Integration

Benchmark results SHALL feed the AutoSpec router.

Example:

```text
task:
  language: go
  type: shared-library-change
  repository_age: legacy
  risk: medium

router:
  candidates:
    model-a: 0.93
    model-b: 0.89
    local-qwen: 0.84
```

Scores MAY incorporate:

```text
capability
availability
quota
latency
cost
historical success
task similarity
hardware
```

---

# 47. Purpose-Specific Profiles

Example:

```yaml
model: qwen-local

capabilities:
  rust_implementation: 0.91
  go_implementation: 0.94
  shell_debugging: 0.96
  scientific_python: 0.83
  textual_architecture_analysis: 0.61
  legacy_planning: 0.66
  review: 0.72
```

The router SHALL NOT infer that excellent Go implementation means excellent architecture planning.

---

# 48. Model Advisory Output

AutoSpec SHOULD be able to produce:

```bash
autospec model advise \
  --repo org/lc-binbase-scheduler \
  --task "diagnose scheduler state recovery"
```

Output:

```text
Planner
  Claude X                95
  GPT X                   93
  Local Model             71

Implementer
  GPT X                   94
  Local Model             92
  Claude X                90

Test Engineer
  Claude X                96
  GPT X                   91

Reviewer
  Claude X                97
  GPT X                   94
```

Reasoning SHALL cite benchmark families rather than arbitrary preference.

---

# 49. CLI Requirements

Minimum CLI:

```bash
autospec bench list
autospec bench show RW-LCBS-0042

autospec bench run RW-LCBS-0042 \
  --model MODEL

autospec bench run-suite realwork-v1 \
  --model MODEL

autospec bench compare \
  --suite realwork-v1 \
  MODEL_A MODEL_B

autospec bench mine \
  --repo OWNER/REPO

autospec bench qualify MODEL

autospec model advise
```

---

# 50. Corpus Layout

Suggested repository structure:

```text
benchmarks/
  realwork/
    schema/
      task-manifest.schema.json

    public/
      autospec/
      mona/
      cts-lite/
      masscube/
      wcmc/

    private/
      lc-binbase-scheduler/
      wcmc-apps/
      go-modules/

    counterfactual/
```

Private benchmark material SHOULD normally live outside the public AutoSpec repository.

---

# 51. Rust Architecture

Core implementation SHOULD live in Rust.

Suggested modules:

```text
crates/autospec-bench/
    src/
      corpus.rs
      manifest.rs
      runner.rs
      evaluator.rs
      scoring.rs
      telemetry.rs
      replay.rs
      mining.rs
      counterfactual.rs
      capability.rs
      qualification.rs
      report.rs
      privacy.rs
```

Possible types:

```rust
pub struct RealWorkTask;
pub struct HistoricalReplay;
pub struct BenchmarkManifest;
pub struct BenchmarkResult;
pub struct CapabilityEvidence;
pub struct RoleScore;
pub struct RepositorySource;
pub struct HiddenGateResult;
```

---

# 52. Immutable Benchmark Identity

A benchmark task SHALL be content-addressable.

Identity SHOULD include:

```text
manifest hash
base commit
environment hash
hidden-test hash
fixture hash
```

This prevents benchmark drift.

---

# 53. Result Ledger

Every run SHALL append to the existing benchmark/routing ledger.

A result SHALL include:

```text
task_id
suite_version
model
provider
role
hardware
quantization
backend

success
score
dimension scores

tokens
tok/s
TTFT
wall time
cost
energy

attempts
repair turns

files inspected
files changed

timestamp
artifact hashes
```

---

# 54. Benchmark Reports

AutoSpec SHALL produce at least:

### Model report

```text
What is this model good at?
```

### Role report

```text
Which models make good reviewers?
```

### Repository-family report

```text
Which models perform best on WCMC legacy apps?
```

### Hardware report

```text
What does local inference cost us?
```

### Regression report

```text
Did Model X become worse after an upgrade?
```

---

# 55. Dashboard Views

The UI SHOULD provide:

```text
Model × Role
Model × Language
Model × Repository Family
Model × Difficulty
Model × Hardware
Quality × Throughput
Quality × Cost
First-pass Success
Legacy Restraint
Cross-repo Diagnosis
```

---

# 56. Minimum Initial Corpus

Before `realwork-v1` is considered usable, it SHALL contain at least:

```text
10 Tier-1 legacy application tasks
5 shared Go-module tasks
5 cross-repository tasks
5 infrastructure tasks
5 scientific/numerical tasks
5 AutoSpec tasks
5 modern full-stack tasks
```

Minimum:

**40 validated historical tasks.**

Each historical task MAY yield multiple role variants, so the actual number of evaluations will be considerably larger.

---

# 57. Target Mature Corpus

A mature suite SHOULD contain:

```text
100+ historical engineering problems
250+ role-specific evaluations
30+ counterfactual cases
20+ cross-repository scenarios
multiple historical architecture generations
```

---

# 58. Seed Tasks Already Identified

The initial public seed set SHOULD include examples already diagnosed:

```text
RW-AS-001
Bats assertion recognition

RW-AS-002
GitHub API read consolidation

RW-AS-003
eventual-consistency-safe GitHub mutation

RW-AS-004
process identity / PID reuse

RW-AS-005
macOS/Linux portability

RW-AS-006
disconnected architecture discovery

RW-AS-007
prompt/context optimization

RW-CTS-001
dataset → Go → API → UI → Playwright vertical slice

RW-CTS-002
bad identifier / service outage resilience

RW-CTS-003
pathological query handling

RW-MONA-001
RabbitMQ consumer recovery

RW-MONA-002
null compound handling

RW-MONA-003
chart lifecycle behavior

RW-MONA-004
lazy-loading lifecycle defect

RW-MC-001
numerical performance optimization

RW-MC-002
NumPy migration
```

Tier-1 WCMC and LC-BinBase tasks SHALL then be added and should ultimately outweigh this seed set.

---

# 59. Private Repository Discovery

Once GitHub/private access exists for:

```text
lc-binbase-scheduler
Go modules
major WCMC applications
```

AutoSpec SHOULD run an inventory pass before task mining.

The inventory SHALL record:

```text
repository
primary languages
age
commit count
PR count
issue count
default branch
build system
test system
CI system
major dependencies
known consumers
deployment relationship
```

This produces the repository dependency graph used by cross-repository benchmarks.

---

# 60. Repository Dependency Graph

AutoSpec SHOULD maintain a graph:

```text
RepositoryGraph
```

Example:

```text
lc-binbase-scheduler
    │
    ├── shared-go-module-A
    │        └── service-X
    │
    ├── database-schema
    │
    └── deployment-config
```

Edges SHOULD have types:

```text
build_dependency
runtime_dependency
API_dependency
deployment_dependency
data_dependency
shared_library
infrastructure
```

The benchmark system MAY intentionally hide graph edges from evaluated models where discovering them is part of the task.

---

# 61. Historical Architecture Knowledge

Benchmark creation MAY store internal architecture metadata.

However:

**architecture metadata SHALL NOT automatically be exposed to the evaluated model.**

Some evaluations explicitly measure whether the model can reconstruct architecture from the repository.

---

# 62. Baseline Models

Every corpus release SHOULD run against:

```text
one strong frontier model
one alternative frontier model
one strong local model
one smaller local model where feasible
```

This validates that tasks discriminate between models and are not simply impossible.

---

# 63. Benchmark Validity

A benchmark SHALL be rejected if:

- baseline is already green;
- failure cannot be reproduced;
- hidden test does not actually discriminate;
- only the exact historical diff can pass;
- benchmark requires unavailable production infrastructure;
- result depends on nondeterministic external APIs;
- acceptance criteria are ambiguous;
- private information leaks.

---

# 64. Human Validation

High-value benchmark tasks SHOULD be reviewed by a human familiar with the repository before becoming part of the calibrated corpus.

Particularly:

```text
LC-BinBase
WCMC infrastructure
scientific algorithms
security-sensitive tasks
```

---

# 65. Corpus Versioning

Versions SHALL be immutable:

```text
realwork-v1.0
realwork-v1.1
realwork-v2.0
```

Adding tasks MAY produce a minor version.

Changing the semantics or expected behavior of an existing task requires a new task ID or appropriate major-version change.

---

# 66. Performance Regression Monitoring

AutoSpec SHOULD rerun a representative benchmark subset whenever:

- a model version changes;
- quantization changes;
- inference backend changes;
- major AutoSpec routing logic changes;
- hardware changes.

Example:

```text
realwork-smoke
```

could contain 10–20 representative cases.

---

# 67. Model Qualification Rules

Production qualification SHOULD require minimum scores.

Example:

```text
implementer:
    correctness >= 0.85
    regression_safety >= 0.90

reviewer:
    defect_detection >= 0.90

legacy_implementer:
    legacy_restraint >= 0.85

cross_repo_diagnoser:
    root_cause_accuracy >= 0.80
```

Exact thresholds SHOULD be configurable and calibrated empirically.

---

# 68. Automatic Degradation

A model's qualification MAY decay when recent benchmark results materially regress.

Example:

```text
calibrated
    ↓
degraded
    ↓
requalification required
```

This matters when cloud providers silently update model aliases or local inference configurations change.

---

# 69. Separation of Duties

Benchmark-derived routing SHALL respect AutoSpec's established separation rules.

The model implementing a work item SHALL NOT be its only planner/reviewer where separation is required.

Capability calibration determines **which eligible model** performs each role.

It does not override independence requirements.

---

# 70. Implementation Phases

## Phase 1 — Corpus Framework

Implement:

- manifests;
- replay runner;
- scoring;
- hidden tests;
- telemetry integration;
- result ledger.

## Phase 2 — Public Seed Corpus

Add:

- AutoSpec;
- MoNA;
- CTS-Lite;
- MassCube;
- WCMC public infrastructure.

## Phase 3 — Private Tier-1 Corpus

Mine:

- LC-BinBase Scheduler;
- WCMC applications;
- Go modules.

This phase becomes the primary production calibration dataset.

## Phase 4 — Cross-Repository Evaluation

Implement:

- repository graph;
- multi-repo workspaces;
- dependency impact scoring;
- hidden root-cause locations.

## Phase 5 — Counterfactual Generation

Create leakage-resistant variants.

## Phase 6 — Role Expansion

Add:

- diagnoser;
- implementer;
- tester;
- reviewer;
- planner;
- documentation;
- UX.

## Phase 7 — Router Integration

Use benchmark evidence directly for task/model selection.

---

# 71. Acceptance Criteria

This specification is complete when all of the following are true:

- [ ] `realwork-v1` exists as a first-class benchmark suite.
- [ ] Historical replay from pinned pre-fix SHAs is supported.
- [ ] Historical solutions are never used as exact patch targets.
- [ ] Hidden behavioral acceptance tests are supported.
- [ ] Public and private corpora are separated.
- [ ] LC-BinBase Scheduler has a dedicated benchmark family.
- [ ] Major WCMC applications are Tier-1 sources.
- [ ] Shared Go modules have a dedicated benchmark family.
- [ ] Cross-repository diagnosis is supported.
- [ ] Repository dependency graphs are supported.
- [ ] Multiple historical generations of long-lived applications are supported.
- [ ] Legacy restraint is measured.
- [ ] Scope discipline is measured.
- [ ] First-pass success and repair cycles are measured.
- [ ] tokens/sec is reported for every evaluation where available.
- [ ] TTFT and wall-clock latency are recorded.
- [ ] hardware and quantization are recorded.
- [ ] cost per successful task is supported.
- [ ] energy per successful task is supported where measurable.
- [ ] numerical tolerance tests are supported.
- [ ] fault-injection tests are supported.
- [ ] role-specific benchmarking is supported.
- [ ] multi-agent/team benchmarking is supported.
- [ ] separation-of-duty rules remain enforced.
- [ ] counterfactual variants are supported.
- [ ] benchmark versions are immutable.
- [ ] capability calibration feeds AutoSpec's router.
- [ ] generic coding benchmarks alone cannot qualify a production model.
- [ ] local and hosted models use the same behavioral evaluation gates.
- [ ] benchmark reporting supports model × role × domain analysis.

---

# 72. Final Architectural Principle

The benchmark system SHALL optimize for this question:

> **Would we trust this model to perform this kind of work in one of our real systems?**

Not:

> Can it solve a generic programming puzzle?

And not:

> How quickly can it emit tokens?

The benchmark must reflect the systems AutoSpec actually needs to maintain:

```text
decades of legacy engineering
+
modern applications
+
shared libraries
+
scientific software
+
infrastructure
+
distributed systems
+
cross-repository dependencies
+
autonomous engineering
```

LC-BinBase Scheduler, the major WCMC applications and the shared Go modules are therefore not supplementary datasets.

**They SHALL constitute the primary production-engineering calibration corpus for AutoSpec.**

The public repositories provide reproducibility and breadth.

The private historical WCMC corpus provides the high-value, leakage-resistant evidence needed to determine which models should actually be trusted with production engineering work.
