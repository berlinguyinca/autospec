# AutoSpec Multi-Model Engineering Team — design

**Date:** 2026-08-16  
**Status:** Implementation specification  
**Target repository:** `berlinguyinca/autospec`  
**Priority:** High  
**Scope:** AutoSpec Run, Autonomous, Fleet, model routing, telemetry, GitHub Projects, testing, documentation, UI/UX verification  
**Relationship to existing work:** Extends `docs/specs/2026-08-05-self-discovering-model-routing-design.md` and the existing AutoSpec Fleet architecture. Do not create an independent competing router.

## Reconciliation with the 2026-08-05 routing design

This document **supersedes** `docs/specs/2026-08-05-self-discovering-model-routing-design.md`
as the authoritative routing design, and absorbs rather than replaces its three layers:

| 2026-08-05 layer | Status here |
|---|---|
| Supply discovery (hardware fingerprint, runtime/model probe, cached capability) | Retained; extended by §7 capability advertisement, §8 evidence levels, §13 provider status |
| Execution path (profile name → real local dispatch) | Retained; generalized into §16 common executor abstraction and §17 local Qwen execution |
| Learned routing (append-only outcome ledger, effective cost, first-pass floor) | Retained; extended by §24–§29 with role, hardware, runtime, quantization, context-band and concurrency dimensions |

Constraints inherited unchanged from the August 5 design and **not** to be reinvented:

- The routing ledger is append-only JSONL mirroring `explore-ledger.sh` (`--stats`,
  Bayesian-smoothed derived weights, `--rebuild`). Do not introduce a second telemetry store.
- Policy exposure follows the existing `advisor:` idiom — one `policy: auto|on|off`
  knob, self-governed from telemetry, no per-gate levers (see §40).
- Existing routing budget hints feed the router; they are not duplicated (§14, Wave 5).

Any implementation issue derived from this spec that would create a parallel router,
a second ledger format, or a per-model config switch matrix is out of scope by
construction and must be rejected in review.

## Superseding program — AS-AEO-001

[`2026-08-16-autonomous-engineering-organization-design.md`](2026-08-16-autonomous-engineering-organization-design.md)
(AS-AEO-001) subsumes the concerns of this document at a higher altitude and in
Rust: roles and separation of duties, capability registry, context management,
provider health and quota, adaptive routing with explanation, executor
abstraction, and the audit ledger all reappear there as §16–§19, §21–§22, §32,
§26, §27–§29, §67 and §58.

This document remains the source of the already-filed issues #3163–#3176, which
implement those behaviors in bash against the routing surface that exists today.
Whether that work continues, is wrapped as an AS-AEO-001 §72.2 compatibility
adapter, or is withdrawn is decided by the AS-AEO-001 §73 Phase 0 migration map —
not by either spec on its own. Until that decision is recorded, do not file new
issues from either document for a subsystem the other already covers.

## Amendments

Two companion specs extend this document and are decomposed alongside it:

| Amendment | Extends | Adds |
|---|---|---|
| [`2026-08-16-benchmark-per-evaluation-telemetry-design.md`](2026-08-16-benchmark-per-evaluation-telemetry-design.md) | §8, §25, §26, §28, §32 | The `autospec bench` harness and the per-evaluation performance telemetry that gives *calibrated* (§8) a concrete measurement contract |
| [`2026-08-16-vision-image-generation-qualification-design.md`](2026-08-16-vision-image-generation-qualification-design.md) | §7, §9, §21, §32, Wave 11 | Vision and image-generation capability classes, their benchmark suites, and the path by which a visual capability moves from advertised to calibrated |

The vision amendment depends on the telemetry amendment for its harness and
metric contract. Neither introduces a routing concept independent of this
document.

---

# 1. Executive Summary

AutoSpec shall evolve from a system that selects an execution tier into a **capability-aware, usage-aware, resource-aware multi-model software engineering team**.

AutoSpec itself remains the deterministic conductor.

Models are workers and specialists.

A typical work item should be able to flow through:

```text
                   AutoSpec Conductor
                          │
                          ▼
                  Planner / Architect
                       Claude
                          │
             ┌────────────┴────────────┐
             ▼                         ▼
       Test Planner               Task Graph
       Claude/Codex                   │
                                      ▼
                              Qwen Implementer
                                      │
                                      ▼
                                Codex Review
                                      │
                       ┌──────────────┼──────────────┐
                       ▼              ▼              ▼
                      QA          UI/UX Vision     Docs
                       │              │              │
                       └──────────────┼──────────────┘
                                      ▼
                                  Merge Gate
```

The actual models used are dynamic.

For example:

```text
Planning             Claude frontier
Implementation       local Qwen
Quick review         Codex Spark
Deep review          frontier Codex
Test planning        Claude
Test verification    Codex
Documentation        local Qwen
Documentation review Claude/Codex
UI/UX verification   vision-capable model
Security review      strongest eligible independent model
```

These are **defaults and priors**, not permanent mappings.

AutoSpec shall learn which models work best for which processes from actual outcomes.

---

# 2. Core Design Principle

AutoSpec chooses a **team**, not merely a model.

A dispatch decision therefore becomes:

```text
work item
   +
engineering role
   +
required capabilities
   +
context requirement
   +
risk
   +
provider availability
   +
quota
   +
hardware capacity
   +
historical model performance
   +
independence constraints
   ↓
eligible candidates
   ↓
utility ranking
   ↓
selected model
```

Provider identity, model identity, execution role, hardware resource and engineering task type must be separate concepts.

---

# 3. Mandatory Engineering Roles

AutoSpec must understand at minimum the following roles:

```text
orchestrator
planner
architect
test_planner
implementer
code_reviewer
test_reviewer
qa_verifier
documentation_writer
documentation_reviewer
ui_ux_reviewer
security_reviewer
researcher
advisor
```

These roles are orthogonal to existing `dispatch_kind`.

For example:

```yaml
dispatch:
  kind: spec-decompose
  role: planner
```

or:

```yaml
dispatch:
  kind: lgtm-reviewer
  role: code_reviewer
```

Existing dispatch kinds should remain useful, including concepts such as:

```text
implementer
lgtm-reviewer
explore-researcher
verify-voter
refine-lens
qa-sweep
secaudit-pass
spec-decompose
```

Role identifies **organizational responsibility**.

Dispatch kind identifies **the work being performed**.

---

# 4. Hard Independence Invariants

These rules are mandatory and centrally enforced by the conductor.

## 4.1 Implementation independence

For every work item:

```text
implementation_model != planning_model
implementation_model != code_review_model
implementation_model != test_review_model
```

A model may not be the sole reviewer of code it implemented.

## 4.2 Planner and reviewer reuse

The same model may perform planning and reviewing if:

```text
planner_model == reviewer_model
```

and:

```text
capability_class(planner_model)
>
capability_class(implementation_model)
```

Example:

```text
Claude      planning
Qwen        implementation
Claude      review
```

is valid.

## 4.3 Escalation changes eligibility

If implementation is escalated from:

```text
Qwen → Codex
```

then Codex immediately becomes ineligible to review that implementation.

Reviewer candidates must be recalculated.

For example:

```text
Claude → planning
Codex  → escalated implementation
Claude → review
```

## 4.4 No self-validation through aliases

Model independence must be based on underlying model identity/version where known, not profile names.

These must not count as independent merely because they have separate aliases:

```text
qwen-fast
qwen-review
```

if they point to the same underlying weights.

## 4.5 Adversarial verification

Existing independent voter/proposer behavior remains mandatory.

A verification voter must not use the same underlying model as the proposal it is evaluating.

## 4.6 Safety gates

Security-sensitive operations may impose stronger requirements:

```text
review capability class >= configured safety floor
```

Local models may be excluded from designated high-risk gates until calibrated.

---

# 5. AutoSpec Conductor

The conductor must primarily be deterministic software.

Do not use a frontier LLM merely to:

- count queue entries;
- inspect quotas;
- reserve context;
- enforce role separation;
- move project cards;
- select candidates from deterministic constraints;
- track dependencies;
- retry known failure classes.

The conductor owns:

```text
task graph
task state
claims
dependencies
role assignments
eligibility
model selection
quota checks
provider health
context reservations
concurrency
retry counters
escalation
project synchronization
telemetry
gate completion
```

A frontier `orchestrator_advisor` model is used only when semantic judgment is genuinely needed.

Examples:

- Should an unexpectedly difficult task be decomposed?
- Is a review failure caused by implementation or flawed planning?
- Does an architectural conflict require replanning?
- Are acceptance criteria contradictory?

---

# 6. Provider vs Model

Provider and model must be separate objects.

Example:

```yaml
provider:
  id: codex
```

may expose:

```text
codex-spark
codex-frontier
other available Codex models
```

Likewise:

```yaml
provider:
  id: bender-local
```

may expose:

```text
qwen-27b-q4
qwen-27b-q6
another local model
```

Do not route simply to:

```text
provider = Codex
```

Route to:

```text
provider + model + execution profile
```

---

# 7. Model Capability Advertisement

Every model must expose a capability document.

Example:

```yaml
schema: autospec.model-capability.v1

provider: bender-local
model: qwen-27b
model_version: detected
profile: qwen-local-27b

capability_class: C

roles:
  planner: false
  architect: false
  test_planner: true
  implementer: true
  code_reviewer: false
  test_reviewer: false
  documentation_writer: true
  documentation_reviewer: false
  ui_ux_reviewer: false
  researcher: true

modalities:
  text: true
  vision: false
  audio: false

automation:
  filesystem: true
  shell: true
  git: true
  browser: false
  computer_use: false
  structured_output: true
  tool_calling: true

specialties:
  implementation: 0.90
  targeted_patch: 0.92
  refactoring: 0.84
  test_generation: 0.93
  merge_resolution: 0.75
  debugging: 0.80
  architecture: 0.55
  documentation: 0.90

languages:
  python: supported
  java: supported
  scala: supported
  rust: supported
  typescript: supported
```

Capability scores are initial priors only.

Observed statistics supersede them once sufficient evidence exists.

---

# 8. Capability Evidence Levels

Capability state progresses through:

```text
advertised
    ↓
discovered
    ↓
calibrated
    ↓
observed
```

## Advertised

Declared by model/runtime/provider.

## Discovered

Confirmed mechanically.

Example:

```text
vision endpoint exists
tool calling enabled
context advertised as 262144
```

## Calibrated

AutoSpec benchmark confirms that capability works.

## Observed

Real AutoSpec task outcomes establish actual production performance.

Routing priority:

```text
observed > calibrated > discovered > advertised
```

---

# 9. Capability Classes

Add capability classes independent of provider.

Suggested initial ordering:

```text
A = frontier reasoning / highest-confidence oversight
B = strong specialist / coding / review
C = commodity local worker
D = experimental / unqualified
```

This is not equivalent to parameter count.

Models must earn capability class by calibration and observation.

A smaller specialist model can outperform a larger general model for a specific process.

---

# 10. Process-Specific Model Selection

AutoSpec must choose models based on the process being performed.

Example prior:

| Process | Preferred type |
|---|---|
| architecture planning | frontier reasoning |
| requirements decomposition | frontier reasoning |
| routine implementation | local coding model |
| test generation | local coding model |
| quick code review | fast specialist |
| merge/conflict work | fast specialist |
| deep cross-module review | frontier reviewer |
| security review | strongest eligible reviewer |
| documentation writing | local model |
| documentation verification | independent stronger model |
| UI/UX review | vision + browser automation |
| routine research | local/cheap model |
| difficult debugging | frontier specialist |

AutoSpec must allow different models within the same provider.

Example:

```text
Codex Spark
    ↓
merge / targeted edit / rapid review

Frontier Codex
    ↓
complex review / difficult debugging
```

Selection must eventually be based on measured data rather than this static prior.

---

# 11. Context Must Be a Schedulable Resource

This is mandatory.

A model's maximum context does not imply every concurrent worker can use that context.

AutoSpec must distinguish:

```yaml
context:
  advertised_max: 262144
  model_verified_max: 262144
  hardware_proven_single_max: 205000
  total_context_capacity: 205000
```

A task provides:

```yaml
context:
  minimum_required: 48000
  preferred: 100000
  maximum_useful: 180000
```

The scheduler reserves context.

Example:

```text
Local GPU context capacity: 200k

worker A: 90k
worker B: 85k
----------------
reserved: 175k
free:      25k
```

Valid configurations may therefore include:

```text
1 × 200k
2 × 100k
4 × 50k
```

when experimentally proven.

The scheduler must not assume linearity.

---

# 12. Context/Concurrency Performance Curves

AutoSpec must measure each local model/hardware combination.

Key:

```text
hardware fingerprint
× runtime
× model
× quantization
× context
× concurrency
```

Example learned table:

| Context | Concurrent | tok/s each | aggregate tok/s | first pass |
|---:|---:|---:|---:|---:|
| 200k | 1 | 27 | 27 | 89% |
| 100k | 2 | 25 | 48 | 92% |
| 50k | 4 | 18 | 67 | 94% |
| 25k | 8 | 9 | 70 | 93% |

Values shown are illustrative.

AutoSpec must create these values from actual measurements.

The existing assumption that a local GPU is universally capacity-1 must be replaced with measured capacity.

---

# 13. Provider Runtime Advertisement

Providers expose operational state separately from model capabilities.

Example local provider:

```yaml
schema: autospec.provider-status.v1

provider: bender-qwen
status: healthy

hardware:
  type: nvidia
  gpu: RTX-4090
  vram_gb: 24

runtime:
  type: llama.cpp
  endpoint: configured

capacity:
  total_context: 205000
  reserved_context: 148000
  free_context: 57000

concurrency:
  active: 2
  preferred: 3
  proven_max: 8

performance:
  aggregate_decode_tok_s: 53.4
  queue_depth: 3
```

Cloud provider:

```yaml
provider: codex

status: healthy

quota:
  usable: true
  state: normal

models:
  - codex-spark
  - codex-frontier
```

---

# 14. Usage and Quota Awareness

Before every dispatch, AutoSpec checks:

```text
provider reachable?
model available?
quota available?
specific model quota available?
required context available?
capacity available?
role permitted?
independence valid?
```

Quota exhaustion is not a fatal workflow error.

It triggers candidate reselection.

Example:

```text
Reviewer preferred: Codex frontier

Codex frontier exhausted
        ↓
Codex Spark eligible?
        ↓
No, deep review capability insufficient
        ↓
Claude eligible?
        ↓
Yes
        ↓
Claude review
```

Quota pools may differ by provider model and must be tracked separately where observable.

Existing budget hints remain useful but should feed the router rather than act as the only availability signal.

---

# 15. Provider Health

Provider health states:

```text
healthy
degraded
quota_limited
overloaded
unreachable
misconfigured
unknown
```

Local model health must include runtime and accelerator health.

Do not silently fall back from GPU to CPU and report the provider healthy.

Fail closed when accelerator state is ambiguous.

---

# 16. Common Executor Abstraction

All models must be accessed through a common execution contract.

Example conceptual interface:

```text
dispatch(request) → result
```

Request includes:

```yaml
work_item:
role:
dispatch_kind:
model:
provider:
context_budget:
tools:
workspace:
acceptance_criteria:
timeout:
```

Result includes:

```yaml
status:
output:
patch:
input_tokens:
output_tokens:
cached_tokens:
prompt_tok_s:
decode_tok_s:
ttft_ms:
wall_clock_ms:
tool_calls:
failure_class:
```

Local Qwen must become a real executor through this abstraction.

Do not special-case local implementations throughout orchestration code.

---

# 17. Local Qwen Execution

Support OpenAI-compatible local inference endpoints.

Initial supported runtimes should include the practical endpoints already contemplated by existing AutoSpec routing design:

```text
llama.cpp server
vLLM
Ollama
OpenCode/Codex-compatible local endpoint where appropriate
```

Runtime discovery and model discovery must be deterministic.

Do not infer installed models from names mentioned in prompts or configuration history.

---

# 18. Task Lifecycle

Recommended canonical state machine:

```text
BACKLOG
  ↓
READY
  ↓
PLANNING
  ↓
TEST_PLANNING
  ↓
IMPLEMENTING
  ↓
CODE_REVIEW
  ↓
QA
  ↓
SPECIALIST_GATES
  ├─ UI_UX
  ├─ DOCS
  └─ SECURITY
  ↓
MERGE_READY
  ↓
DONE
```

Additional states:

```text
BLOCKED
ESCALATED
WAITING_QUOTA
WAITING_CAPACITY
REPLAN_REQUIRED
```

Not every task requires every specialist gate.

Gate requirements are determined by task characteristics.

---

# 19. Test Planning

Test planning is a first-class role.

The feature planner supplies:

```text
requirements
acceptance criteria
architecture constraints
```

The test planner independently produces:

```text
happy paths
edge cases
failure cases
regression risks
integration tests
system tests
required manual/visual validation
```

This should happen before implementation when practical.

The implementer may write tests but does not control the complete definition of success.

---

# 20. Test Review / QA

After implementation, an independent model reviews:

```text
implementation
tests
test plan
acceptance criteria
actual test results
```

The reviewer should ask:

- Are important scenarios missing?
- Do tests merely encode the implementation's assumptions?
- Are negative paths covered?
- Are regressions covered?
- Are assertions meaningful?
- Were all required test suites actually run?

The implementation model cannot be the sole test reviewer.

---

# 21. UI/UX Verification

UI/UX validation is a mandatory capability-driven specialist pipeline when the work changes user-visible interfaces.

Required capabilities:

```yaml
modalities:
  vision: true

automation:
  browser: true
```

Preferred additional capabilities:

```text
computer_use
screenshot_analysis
accessibility analysis
responsive viewport control
```

Workflow:

```text
start application
    ↓
navigate scripted user journeys
    ↓
capture screenshots
    ↓
exercise interactions
    ↓
vision analysis
    ↓
compare against requirements/design
    ↓
report visual/interaction defects
```

Required checks include:

```text
layout
overflow/clipping
spacing
alignment
responsive behavior
loading states
error states
forms
navigation
keyboard behavior
obvious accessibility defects
visual regressions
```

Where design references exist, screenshots should be compared against them.

A text-only model is ineligible for final UI/UX visual verification.

---

# 22. Documentation Team

Documentation requires separate writer and verifier roles.

## Writer

Routine documentation should preferentially use an inexpensive/local model.

Examples:

```text
README updates
CLI reference
API reference
runbooks
examples
migration notes
CHANGELOG
operator documentation
```

## Reviewer

A different model verifies documentation against:

```text
actual CLI output
source code
configuration schema
implemented behavior
tests
```

The documentation reviewer specifically looks for hallucinated:

```text
flags
endpoints
config values
commands
defaults
behavior
```

---

# 23. Security Reviewer

Security-sensitive tasks require a separate security review role.

Triggers may include:

```text
authentication
authorization
secrets
cryptography
production access
database migrations
destructive operations
network exposure
dependency trust
CI/CD privilege
```

Security review should have a configurable minimum capability class.

---

# 24. Routing Statistics

Statistics must not be recorded only by provider.

Primary performance dimensions:

```text
provider
model
model_version
hardware fingerprint
runtime
quantization
role
dispatch_kind
task capability
language
repository
subsystem
context band
concurrency
difficulty
risk
```

Avoid fragmentation by using hierarchical aggregation and Bayesian smoothing.

---

# 25. Required Per-Dispatch Telemetry

Record at minimum:

```yaml
dispatch_id:
work_item:
provider:
model:
model_version:
profile:

role:
dispatch_kind:

hardware_fingerprint:
runtime:
quantization:

context_requested:
context_reserved:
context_used:

concurrency_at_start:
queue_depth_at_start:

input_tokens:
output_tokens:
cached_tokens:

prompt_tok_s:
decode_tok_s:
aggregate_decode_tok_s:
ttft_ms:
wall_clock_ms:

retry_index:
escalated:
previous_model:

outcome:
review_outcome:
tests_outcome:
merged:
reverted:
```

For cloud providers where some metrics are unavailable, fields may be `unknown`.

Unknown is preferable to fabricated values.

---

# 26. Outcome Statistics

AutoSpec should derive:

```text
first-pass success rate
eventual success rate
mean retries
median retries
escalation rate
review rejection rate
test failure rate
revert rate
median completion time
P95 completion time
prompt tok/s
decode tok/s
aggregate tok/s
cache ratio
cost per successful task
GPU minutes per successful task
```

Statistics must be selectable/filterable by process and model.

Example:

```text
Qwen
  implementation
    Scala
      ctx 32–64k
      ctx 64–128k
      ctx 128–256k
```

---

# 27. Advertised Capability vs Observed Capability

Suppose a model advertises:

```text
architecture = excellent
```

but real AutoSpec statistics show:

```text
architecture first-pass = 54%
```

Observed data wins.

Conversely, a model advertised merely as a coding assistant may prove exceptionally good at:

```text
merge conflict resolution
```

and should become preferred for that process.

---

# 28. Model Performance Ledger

Extend/reuse the routing ledger concept rather than introducing an unrelated telemetry store.

The existing routing design already proposes an append-only ledger with profile, model, harness, dispatch kind, context/reasoning, token counts, wall-clock, retries, escalation and outcome.

Extend that contract with the dimensions in this specification.

The ledger must support:

```text
append
update outcome
stats
filter
validate
rebuild
```

and must survive local machine loss where reconstructable from GitHub/telemetry history.

---

# 29. Learned Utility Routing

Candidate selection has two phases.

## Phase A — eligibility

A candidate is excluded when:

```text
provider unavailable
quota unavailable
model unavailable
missing required modality
missing required tools
context insufficient
capacity unavailable
role forbidden
independence violated
capability below required floor
security floor violated
```

## Phase B — utility ranking

Among eligible candidates, score:

```text
quality
+
success probability
+
throughput
+
latency suitability
+
quota conservation
+
cost
+
queue delay
+
retry probability
+
escalation probability
```

Conceptually:

```text
utility =
    expected_success_value
    - expected_cost
    - expected_delay
    - retry_penalty
    - escalation_penalty
```

Do not route solely on sticker token price.

---

# 30. Foreground vs Background Optimization

AutoSpec must distinguish interactive and asynchronous work.

For interactive work:

```text
latency / tok/s per request
```

may dominate.

For autonomous worker farms:

```text
aggregate successful work throughput
```

is more important.

Therefore:

```text
8 workers × 9 tok/s
```

may be preferable to:

```text
1 worker × 30 tok/s
```

when the tasks are independent and completion quality is comparable.

---

# 31. Queue-Aware Local Scheduling

Local scheduling should use:

```text
free context
active context
queue depth
aggregate tok/s
per-worker tok/s
estimated completion time
success history
```

Do not route additional local work when marginal throughput collapses below an empirically determined threshold.

---

# 32. Calibration

A newly discovered model is not immediately trusted for important roles.

Calibration should replay known historical tasks in isolated worktrees.

Example:

```text
small implementation
test generation
refactor
merge conflict
documentation update
```

Evaluate with existing repository gates.

Possible result:

```text
model qualified for:
  implementation
  docs

model not qualified for:
  planning
  review
```

This is a valid successful calibration result.

---

# 33. Cold Start / Exploration

Unknown models need bounded opportunities to gather evidence.

Allowed approaches:

```text
low-risk epsilon exploration
shadow execution
historical replay
```

Never use unqualified models for security or critical independent review simply to collect statistics.

---

# 34. GitHub Project/Kanban Is Mandatory

Every repository managed by AutoSpec must have an AutoSpec-compatible GitHub Project or explicitly configured shared project.

This is not optional visualization.

It is the human-readable control plane.

AutoSpec's internal conductor remains authoritative.

GitHub Project state mirrors workflow state.

Dragging a card manually must never bypass required engineering gates.

---

# 35. Mandatory GitHub Project Views

At minimum create/maintain:

## Work Board

```text
Backlog
Ready
Planning
Test Planning
Implementing
Code Review
QA
Specialist Verification
Merge Ready
Blocked
Done
```

## Agent / Model Board

Group by selected implementation/provider/model.

Example:

```text
Qwen
Codex Spark
Codex Frontier
Claude
```

## Quality Board

Visualize:

```text
Planning
Test Plan
Code Review
QA
UI/UX
Documentation
Security
```

## Blocked / Escalated

Show:

```text
quota
provider unavailable
context unavailable
dependency
failed test
review rejected
replan required
```

## Roadmap

Show epics, child issues, dependencies, priority and progress.

---

# 36. GitHub Project Custom Fields

Recommended required fields:

```text
Status
AutoSpec Role
Planner Provider
Planner Model
Implementer Provider
Implementer Model
Reviewer Provider
Reviewer Model
Test Planner
Test Reviewer
UI/UX Reviewer
Documentation Writer
Documentation Reviewer
Security Reviewer
Risk
Difficulty
Context Minimum
Context Preferred
Context Reserved
Capability Requirements
Attempt Count
Escalation Count
Blocked Reason
Current Provider
Current Model
Queue State
Estimated Cost
Actual Cost
Wall Clock
First Pass
Quality Gates
Epic
Dependency State
```

Not every field has to be visible in every view.

---

# 37. Project Synchronization

When AutoSpec:

```text
creates an issue
adopts an issue
plans work
claims implementation
opens a PR
requests review
fails review
runs QA
requires UI review
requires docs review
becomes blocked
escalates
merges
```

the GitHub Project must update automatically.

Synchronization errors must be visible.

Do not silently lose project state.

---

# 38. Project Source of Truth

Authoritative state:

```text
AutoSpec conductor state
```

Visualization:

```text
GitHub Project
```

GitHub Project changes may request transitions but must pass conductor validation.

Example:

Manually moving an issue to Done does **not** satisfy:

```text
tests
review
security
UI
documentation
```

requirements.

---

# 39. Epic and Sub-Issue Support

AutoSpec should create proper epic/sub-issue structures for larger plans.

Implementation specs should be decomposed into independently reviewable issues.

The Project roadmap must reflect these relationships.

---

# 40. Proposed Configuration

Add a top-level routing block.

Example:

```yaml
routing:
  policy: auto

  independence:
    require_distinct_implementation_model: true
    allow_planner_as_reviewer: true
    require_stronger_planner_reviewer: true

  local:
    enabled: true
    calibration_required: true

  learning:
    enabled: true
    min_samples: 10

  context:
    reservations: true
    dynamic_concurrency: true

  availability:
    quota_checks: true
    provider_health_checks: true

  project:
    required: true
```

Do not expose dozens of per-model micromanagement switches unless necessary.

Prefer intent + safety bounds + learned behavior.

---

# 41. Provider Configuration

Example:

```yaml
providers:

  claude:
    type: cloud
    discovery: automatic

  codex:
    type: cloud
    discovery: automatic

  qwen-bender:
    type: openai-compatible
    endpoint: http://configured-host/v1
    discovery: automatic
```

Secrets must remain external.

---

# 42. GitHub Project Configuration

Example:

```yaml
project:
  required: true
  mode: auto
  owner: auto
  project_number: auto

  views:
    work: true
    agents: true
    quality: true
    blocked: true
    roadmap: true
```

If AutoSpec lacks permission to create/update the required project, startup must report a clear degraded/error state according to policy.

---

# 43. Failure and Fallback Rules

A failed candidate selection must produce a reason.

Examples:

```text
codex-spark:
  rejected: context_too_small

codex-frontier:
  rejected: quota_exhausted

qwen:
  rejected: same_model_as_implementer

claude:
  selected
```

These decisions must be logged and explainable.

---

# 44. Wait vs Fallback

Do not always fall back immediately.

The router should distinguish:

```text
fallback now
wait briefly for capacity
park until quota reset
escalate to operator
```

Example:

A routine background Qwen implementation may wait for local capacity instead of consuming scarce frontier quota.

A merge-blocking review may use an eligible cloud fallback immediately.

---

# 45. Model Selection Explainability

Provide a command or status output similar to:

```text
autospec routing explain <issue>
```

Expected explanation:

```text
Role: code_reviewer
Required context: 72k
Required capability: review >= B

Rejected:
- qwen-local: implementation model for this work item
- codex-spark: review quality below threshold for deep cross-module task

Eligible:
- codex-frontier: quota exhausted
- claude: available, class A, success rate 96%

Selected:
claude
```

---

# 46. Project Visualization of Scheduler State

Where useful, expose local resource status:

```text
Qwen / Bender
Active workers: 3
Reserved context: 148k / 205k
Queue: 2
Aggregate decode: 56 tok/s
```

This may be shown through project fields, status summaries or generated project dashboards.

Do not generate excessive GitHub writes every second.

Updates should be event-driven or suitably throttled.

---

# 47. Test Requirements

Every foundational routing component requires deterministic tests.

## Independence

Test:

```text
planner=A
implementer=B
reviewer=A
```

accepted.

Test:

```text
planner=A
implementer=A
reviewer=B
```

rejected.

Test escalation changing implementation and reviewer eligibility.

## Context

Test:

```text
capacity=200k
reservation=100k
reservation=100k
```

accepted.

Third 50k rejected or queued.

## Quota

Preferred provider unavailable → correct fallback.

## Capability

Vision-required dispatch rejects text-only models.

## Model-family selection

Fast specialist selected for eligible small merge task.

Frontier specialist selected when context or capability exceeds fast model.

## GitHub Project

Every state transition updates project state.

Invalid manual transition cannot bypass gates.

## Telemetry

Every dispatch records required fields or explicit `unknown`.

---

# 48. Integration Tests

Create scenarios covering:

### Scenario A — normal local implementation

```text
Claude plan
Qwen implement
Codex review
QA pass
merge
```

### Scenario B — Codex quota exhausted

```text
Claude plan
Qwen implement
Codex unavailable
Claude review
merge
```

### Scenario C — Qwen failure escalation

```text
Claude plan
Qwen implement
Qwen retry
Codex implement escalation
Claude review
```

### Scenario D — context saturation

Two Qwen workers reserve 100k each.

Third task queues or routes elsewhere.

### Scenario E — UI task

Text-only reviewer is rejected.

Vision/browser-capable reviewer performs UI gate.

### Scenario F — docs

Qwen writes documentation.

Independent model verifies it.

### Scenario G — project synchronization

Issue travels from planning through completion and every Project field/view remains consistent.

---

# 49. Observability

Status commands should expose:

```text
providers
models
health
quota
queues
context reservations
workers
tok/s
task assignments
review assignments
blocked work
recent fallbacks
recent escalations
```

Fleet status should aggregate this across nodes.

This extends the existing fleet concept in which node-local capacity is distinct from shared desired state and the fleet supervisor coordinates repositories while per-repository AutoSpec owns issue execution.

---

# 50. Fleet Integration

Node-local capability becomes richer than a list of profiles.

Example:

```yaml
node_id: bender

providers:
  - qwen-bender

resources:
  context_scheduler: true

max_parallel_repos: 3
```

Fleet chooses where work can run.

Per-repo conductor chooses which issue/model/role executes.

Preserve the existing responsibility boundary:

```text
Fleet
  → node/repository placement

AutoSpec Run
  → issue/task/model execution
```

---

# 51. Security

Provider/model discovery must never log secrets.

Configuration contains references only.

Local endpoint configuration must not expose an unauthenticated inference endpoint outside intended network boundaries by default.

GitHub project automation must use minimum necessary permissions.

Model-generated capability advertisements are untrusted input until calibrated.

---

# 52. Backward Compatibility

Existing users without local models must continue to work.

If advanced routing is unavailable:

```text
fallback to current known-safe cloud behavior
```

Do not break single-provider operation.

Existing profile configurations should be migratable.

---

# 53. Bootstrap Implementation Policy

The first routing foundation must not be implemented primarily by the model whose trust is being established.

Recommended bootstrap:

```text
Planner:       Claude frontier
Implementer:   frontier Codex
Reviewer:      Claude frontier
```

or equivalent independent frontier pairing.

Qwen becomes eligible after the foundational invariants and executor path pass tests.

---

# 54. Implementation Sequence

## Wave 0 — Specification integration

1. Reconcile this spec with the existing August 5 routing design.
2. Update that design or supersede it explicitly.
3. Avoid parallel routing concepts.

**Implementation:** frontier model  
**Review:** independent frontier model

---

## Wave 1 — Engineering role/domain model

Implement:

```text
roles
model identity
provider identity
capability classes
work-item role history
independence rules
```

Add deterministic validation.

**Implementation:** Codex frontier  
**Planning/review:** Claude

---

## Wave 2 — Mandatory GitHub Project control plane

Implement:

```text
project discovery/creation
required custom fields
standard views
issue adoption
state synchronization
epic/sub-issue visualization
```

Make Project integration mandatory for AutoSpec-managed work according to configured policy.

**Implementation:** Codex  
**Review:** Claude

---

## Wave 3 — Capability and provider advertisements

Add:

```text
model capability schema
provider status schema
model discovery
runtime discovery
capability evidence levels
```

**Implementation:** Codex  
**Review:** Claude

---

## Wave 4 — Context and concurrency scheduler

Implement:

```text
advertised context
verified context
hardware context budget
per-task minimum/preferred context
reservations
queueing
dynamic concurrency
```

**Implementation:** Codex  
**Review:** Claude

---

## Wave 5 — Quota, health and availability

Implement provider-specific:

```text
health
usage
quota
rate-limit state
model availability
fallback reasons
```

Integrate existing routing-budget hints rather than duplicating them.

**Implementation:** Codex  
**Review:** Claude

---

## Wave 6 — Deterministic eligibility router

Implement:

```text
role
capability
context
availability
quota
independence
security floor
```

Produce explainable candidate rejection/selection.

No learned ranking required yet.

**Implementation:** frontier Codex  
**Review:** Claude

---

## Wave 7 — Common executor + local Qwen

Implement provider-neutral executor abstraction.

Wire real local Qwen execution.

Test with actual local runtime.

After this wave, Qwen may become an implementation candidate.

**Implementation:** Codex  
**Review:** Claude

---

## Wave 8 — Telemetry and statistics

Extend routing ledger.

Record:

```text
process
model
context
concurrency
tokens/s
wall-clock
retry
review
tests
outcome
```

This is a good first substantial Qwen implementation candidate.

**Implementation:** Qwen  
**Review:** Codex/Claude

---

## Wave 9 — Test planner and QA team

Implement independent:

```text
test planner
test reviewer
QA gate
```

**Implementation:** Qwen where suitable  
**Review:** Codex/Claude

---

## Wave 10 — Documentation team

Implement:

```text
documentation writer
documentation reviewer
documentation correctness gate
```

Prefer Qwen for routine writing.

---

## Wave 11 — UI/UX vision pipeline

Implement:

```text
browser automation
screenshots
vision model dispatch
interaction journeys
responsive checks
accessibility checks
visual gate
```

Must dynamically select a model satisfying vision/browser capabilities.

---

## Wave 12 — Specialist model selection

Support multiple models within each provider.

Enable task-specific selection such as:

```text
fast Codex model → merge/quick patch/review
frontier Codex → difficult/deep review
```

Use capability metadata initially.

---

## Wave 13 — Learned router

Implement empirical utility ranking from the routing ledger.

Use:

```text
success
retry
latency
throughput
cost
queue
quota
context
```

---

## Wave 14 — Adaptive scheduler

Learn optimal:

```text
context × concurrency
```

curves for each local model/hardware/runtime combination.

Route autonomous work for maximum successful aggregate throughput.

---

# 55. Proposed Issue Breakdown

Create an epic:

> **Multi-model engineering team routing, capability discovery, resource scheduling, and GitHub Projects control plane**

Child issues:

1. Add engineering roles and hard model-independence invariants.
2. Add mandatory GitHub Project/Kanban control plane.
3. Add provider/model capability advertisement schemas.
4. Add provider/model discovery and calibration state.
5. Add dynamic context-budget and concurrency reservations.
6. Add provider quota, usage and health probes.
7. Build deterministic capability/availability/independence router.
8. Add provider-neutral executor abstraction.
9. Wire local Qwen execution.
10. Extend routing telemetry and multidimensional statistics.
11. Add test-planner and independent QA roles.
12. Add documentation writer and documentation verifier.
13. Add vision/browser-driven UI/UX verification.
14. Add model-within-provider specialist selection.
15. Add learned utility routing.
16. Add adaptive context/concurrency scheduler.
17. Add fleet-level model/resource aggregation.
18. Add routing explainability/status tooling.
19. Add end-to-end multi-model dogfood suite.

Each issue should define explicit dependencies.

---

# 56. Dogfooding Milestones

## Milestone 1

AutoSpec infrastructure built manually with frontier cross-review.

```text
Claude → plan/review
Codex  → implementation
```

## Milestone 2

Qwen becomes eligible for low-risk implementation.

```text
Claude → plan
Qwen   → implementation
Codex  → review
```

## Milestone 3

AutoSpec selects among models automatically.

## Milestone 4

AutoSpec learns task/model suitability from actual repository outcomes.

## Milestone 5

AutoSpec runs as a continuously improving multi-model engineering organization.

---

# 57. Definition of Done

This project is complete when AutoSpec can take a sufficiently specified issue and autonomously:

1. place it into the mandatory GitHub Project;
2. classify its engineering needs;
3. assign an eligible planner;
4. define acceptance criteria;
5. create an independent test plan where appropriate;
6. select an implementer based on capability, context, quota and capacity;
7. reserve local context/resources;
8. execute implementation;
9. independently review the implementation;
10. independently evaluate tests;
11. invoke UI/UX, documentation or security specialists when required;
12. fall back when providers exhaust quota;
13. recalculate independence after escalation;
14. synchronize all states into GitHub Projects;
15. record model/process performance statistics;
16. merge only after all required gates pass;
17. use accumulated evidence to improve future model selection.

The operator must be able to answer at any time:

```text
What is AutoSpec working on?
Who planned it?
Which model is implementing it?
Who will review it?
Why was that model chosen?
What is blocked?
What quota remains?
How much local context is reserved?
How fast is local inference running?
Which models perform best for this kind of task?
Which quality gates remain?
```

without reconstructing this information manually from logs.

---

# 58. Non-Goals

Do not initially:

- build a wholly new centralized fleet control service;
- replace GitHub as AutoSpec's collaborative work state;
- invent a proprietary Kanban UI when GitHub Projects already provides the human visualization layer;
- require every model to support every role;
- optimize purely for minimum token cost;
- trust model names or parameter count as capability;
- permit implementation models to approve themselves;
- route UI/UX visual validation to text-only models;
- assume local inference concurrency is one;
- assume advertised context equals usable concurrent context.

---

# 59. Architectural North Star

The intended end state is:

```text
                         AUTOSPEC
                    deterministic conductor
                            │
                ┌───────────┼───────────┐
                │           │           │
                ▼           ▼           ▼
             PLANNING   IMPLEMENTATION  QUALITY
                │           │           │
              Claude      Qwen farm     Codex
                │           │           │
                │           │      ┌────┼───────────┐
                │           │      │    │           │
                │           │     QA   Docs       UI/UX
                │           │                   Vision
                │           │
                └───────────┼───────────────┐
                            │               │
                            ▼               ▼
                     learned routing    GitHub Project
                            │             control room
                            ▼
                    better next decision
```

AutoSpec should behave less like “an AI that writes code” and more like a **software engineering organization whose workers happen to be models**.

The conductor manages responsibilities, independence, resources and evidence.

Models compete for work based on demonstrated capability.

Cheap/local models perform high-volume work.

Fast specialist models handle tasks where latency matters.

Frontier models are reserved for work where their additional capability produces measurable value.

Every important artifact is independently validated.

Every decision is observable.

Every outcome makes the next routing decision better.