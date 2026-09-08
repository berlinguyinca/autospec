# AutoSpec Continuous Improvement Engine

Implementation Specification for autospec-define

Status: Proposed
Target: AutoSpec / InferWeave
Primary consumer: autospec-define
Spec type: Architecture + implementation specification
Priority: High

---

## 1. Executive Summary

AutoSpec currently orchestrates planning, implementation, review, testing, model routing, and quality control. However, the system does not yet systematically learn from historical agent execution.

This specification introduces an **AutoSpec Continuous Improvement Engine**: a subsystem that ingests historical coding-agent sessions, CI results, review findings, user corrections, tool activity, and git outcomes; converts them into structured telemetry; detects recurring inefficiencies and failure patterns; and produces evidence-backed proposals to improve AutoSpec itself.

The central objective is:

> AutoSpec should continuously measure how its agents work, discover recurring sources of waste or failure, propose concrete improvements, and verify whether accepted changes actually improve future outcomes.

The system MUST NOT silently rewrite its own instructions, skills, routing rules, tools, or code.

All self-improvement follows a controlled lifecycle:

```text
Observation -> Evidence -> Hypothesis -> Proposed Change -> Evaluation
 -> Pull Request -> Independent Review -> Merge -> Post-change Measurement
 -> Keep / Revert / Retire
```

The Continuous Improvement Engine should operate largely asynchronously and should prefer inexpensive/local inference models for bulk semantic analysis. Stronger models should only analyze aggregated findings, generate high-value recommendations, or independently review proposed changes.

---

## 2. Goals

The implementation MUST enable AutoSpec to:

1. Ingest historical sessions from Pi and be architected for Codex, Claude Code, OpenCode, CI, and additional agent systems.
2. Extract deterministic telemetry before using an LLM.
3. Identify repeated: user corrections; reviewer corrections; tool failures; unnecessary repository exploration; unnecessary git/history exploration; context bloat; architectural mistakes; code-quality violations; test failures; hallucinated APIs or files; unused skills/extensions; inefficient model selection; inefficient tool selection.
4. Treat user re-steering as a first-class quality signal.
5. Measure tool, skill, extension, prompt, and model effectiveness.
6. Correlate agent sessions with git commits, PRs, CI runs, review findings, and outcomes.
7. Generate improvement proposals backed by evidence.
8. Evaluate proposed changes before adoption.
9. Measure post-change impact.
10. Integrate findings and trends into the existing AutoSpec / InferWeave CI dashboard.
11. Support local InferWeave models for low-cost bulk analysis.
12. Avoid unbounded growth of AGENTS.md, skills, prompts, and agent policy.
13. Preserve auditability of every recommendation and accepted change.

---

## 3. Non-Goals

The first implementation MUST NOT:

- Fine-tune foundation models.
- Automatically merge self-modifying changes.
- Allow session-analysis agents to directly alter production agent configuration.
- Build a full reinforcement-learning system.
- Depend on one LLM vendor.
- Require all historical sessions to fit into a single model context.
- Treat every user correction as universally applicable policy.
- Automatically convert isolated incidents into global rules.
- Automatically delete skills or extensions without review.
- Make optimization decisions using only subjective LLM judgments.

---

## 4. Core Design Principles

### 4.1 Deterministic First, Semantic Second

Facts that can be extracted mechanically MUST be extracted mechanically: token counts; tool calls; tool failures; commands executed; files read; files changed; session duration; context growth; git commits; test status; CI results; PR review events; model used; provider used; retries; session branching; exit state.

LLMs SHOULD be used for interpretation such as: task classification; failure classification; user re-steering classification; architecture complaint classification; semantic similarity between recurring failures; explanation of why a pattern matters.

This separation reduces hallucination and improves reproducibility.

### 4.2 Evidence Before Policy

No rule should be proposed merely because an LLM thinks it sounds useful.

Every recommendation SHOULD contain: supporting sessions; exact evidence; occurrence count; affected repositories; affected models; affected task classes; severity; estimated impact; confidence; proposed remediation; risks.

### 4.3 Local Models for Bulk Work

Bulk session analysis SHOULD run on local InferWeave capacity where possible.

```text
Raw Sessions -> Deterministic Extraction -> Local Semantic Enrichment
 -> Structured Records -> Clustering / Aggregation -> Strong-model Diagnosis
 -> Improvement Proposal
```

This avoids repeatedly sending millions or billions of historical tokens to expensive remote models.

### 4.4 Strong Models Operate on Aggregates

```text
500 raw sessions -> 500 structured summaries -> deterministic grouping
 -> 12 recurring patterns -> strong-model analysis
```

### 4.5 No Silent Self-Modification

The engine MAY: create findings; draft skills; draft prompt modifications; draft routing changes; draft code changes; open issues; open pull requests; recommend removal of unused components.

The engine MUST NOT silently: change production configuration; edit production AGENTS.md; alter routing policies; install/remove skills; merge code; disable quality gates; modify review policy.

---

## 5. High-Level Architecture

```text
                     AutoSpec Orchestrator
                              |
          +-------------------+-------------------+
          |                   |                   |
       Planner            Executor             Reviewer
          |                   |                   |
          +-------------------+-------------------+
                              |
                        Agent Sessions
                              |
                              v
                     Session Event Store
                              |
              +---------------+---------------+
              |               |               |
              v               v               v
       Deterministic      Semantic         Git / CI
         Extractor        Enricher         Correlator
              |               |               |
              +---------------+---------------+
                              |
                              v
                        Analytics Store
                              |
          +-------------------+-------------------+
          |                   |                   |
          v                   v                   v
     Pattern Engine      ROI Analyzer      Routing Analyzer
          |                   |                   |
          +-------------------+-------------------+
                              |
                              v
                      Improvement Engine
                              |
                              v
                        Evaluation Gate
                              |
                              v
                         PR / Review
                              |
                              v
                       Post-change Metrics
```

---

## 6. Session Ingestion Service

Responsibilities: discover session sources; parse raw session records; normalize providers into a common event schema; preserve raw source references; support incremental ingestion; prevent duplicate ingestion.

Initial adapters: Pi JSONL sessions; AutoSpec orchestration events; AutoSpec CI events; Git and PR metadata.

Future adapters: Codex; Claude Code; OpenCode; external CI systems; additional coding harnesses.

```go
type SessionAdapter interface {
    Discover(ctx context.Context) ([]SessionRef, error)
    Read(ctx context.Context, ref SessionRef) (<-chan RawEvent, error)
    Normalize(event RawEvent) ([]NormalizedEvent, error)
}
```

---

## 7. Normalized Session Event Model

```json
{
  "event_id": "evt_123",
  "session_id": "session_456",
  "parent_session_id": null,
  "timestamp": "2026-09-08T18:00:00Z",
  "repo": "inferweave/autospec",
  "branch": "feature/foo",
  "work_item_id": "issue-721",
  "agent_role": "implementer",
  "provider": "local",
  "model": "qwen3",
  "event_type": "tool_call",
  "tool": "shell",
  "payload": {},
  "tokens": { "input": 1234, "output": 212 }
}
```

Core event types: `session_started`, `session_finished`, `model_selected`, `model_fallback`, `user_message`, `assistant_message`, `tool_call`, `tool_result`, `tool_error`, `file_read`, `file_write`, `file_patch`, `command_run`, `command_failed`, `test_run`, `test_result`, `lint_result`, `review_result`, `context_compaction`, `context_limit_warning`, `subagent_spawned`, `subagent_finished`, `git_commit`, `pull_request_opened`, `pull_request_reviewed`, `ci_started`, `ci_finished`, `issue_linked`, `user_intervention`.

---

## 8. Session Summary Record

```json
{
  "session_id": "session_456",
  "task_type": "implementation",
  "task_domain": ["go", "concurrency"],
  "outcome": "success",
  "autonomy_score": 0.89,
  "user_interventions": 1,
  "review_rework_count": 2,
  "tool_calls": 93,
  "tool_errors": 4,
  "files_read": 18,
  "files_changed": 6,
  "tests_run": 7,
  "context_peak_tokens": 84220,
  "input_tokens": 118222,
  "output_tokens": 14421,
  "estimated_cost": 0.00,
  "duration_seconds": 551,
  "models": ["qwen3"],
  "commits": ["abc123"],
  "pull_requests": [721]
}
```

---

## 9. User Re-Steering Detection

User re-steering MUST be tracked as a first-class metric. Examples: "Don't create another abstraction."; "Use the existing service."; "Stop adding getters and setters."; "Run the tests first."; "Don't rewrite this."; "You're looking in the wrong directory."; "We already have a helper for this."; "Don't use that model."; "Why are you searching git history?"

Suggested categories:

```text
architecture, scope, tool_selection, context_selection, test_process,
code_quality, model_selection, requirements_misunderstanding,
repository_navigation, unnecessary_work, hallucination, style,
documentation, security, performance, other
```

```json
{
  "session_id": "...",
  "category": "architecture",
  "severity": 3,
  "message_ref": "...",
  "semantic_summary": "Agent introduced unnecessary abstraction",
  "resolved_in_session": true
}
```

---

## 10. Pattern Detection Engine

The engine MUST identify recurring patterns across sessions, generated using deterministic thresholds; time-series changes; semantic clustering; repository grouping; model grouping; task grouping; reviewer findings; user intervention classes.

```text
Pattern: Agents introduce unnecessary abstraction layers.
Occurrences: 17   Sessions: 9   Repositories: 3
Models: Qwen 12, Codex 3, Claude 2
Task classes: implementation 14, refactor 3
Impact: High review rework, +14% tokens, +1.7 turns/task
```

---

## 11. Finding Schema

```json
{
  "finding_id": "finding_001",
  "type": "recurring_architecture_problem",
  "title": "Unnecessary abstraction creation",
  "status": "active",
  "first_seen": "...",
  "last_seen": "...",
  "occurrences": 17,
  "sessions": ["..."],
  "repositories": ["..."],
  "models": { "qwen3": 12, "codex": 3, "claude": 2 },
  "confidence": 0.91,
  "severity": "high",
  "estimated_cost": { "extra_tokens": 192000, "extra_minutes": 88 },
  "evidence": [],
  "candidate_remediations": []
}
```

---

## 12. Finding Lifecycle

```text
candidate -> active -> acknowledged -> proposal_created -> fix_in_progress
 -> monitoring -> resolved | regressed | dismissed
```

The engine SHOULD distinguish newly emerging problems; persistent problems; improving problems; resolved problems; regressions.

Recent observations SHOULD normally weigh more heavily than old observations.

---

## 13. Tool ROI Analysis

Metrics: invocation count; success rate; downstream task success; tool error rate; average added tokens; average saved turns; correlation with rework; frequency of user correction after use; model compatibility; task compatibility.

```text
Tool                  Calls  Success  Errors  Est. ROI
-------------------------------------------------------
LSP                    391     94%      2%       +++
browser-test            23     91%      4%       ++
legacy-java-helper       2     50%     50%        -
git-history-scan       142      8%     12%       ---
```

---

## 14. Removal / Retirement Candidates

```text
REMOVE CANDIDATE
skill: java-legacy-helper
Installed: 73 days   Invoked: 1   Useful outcomes: 0
Estimated context overhead: 1,870 tokens/session
Estimated savings: 130k tokens/week
```

Removal MUST still require review.

---

## 15. Context Intelligence

The system SHOULD learn which context is useful for which task classes.

```text
Task: Add REST endpoint
Usually useful:      controller/, service/, dto/, persistence/, OpenAPI spec
Usually unnecessary: frontend/, deployment/, historical git logs
```

This SHOULD eventually form a repository-specific context graph. The Scout/Context agent SHOULD query this graph before broad repository exploration.

---

## 16. Context Waste Metrics

Track: files read but never referenced again; directories enumerated without downstream use; git history searches without impact; repeated file reads; redundant tool invocations; context added before first useful edit; context compactions; context exhaustion; token count at first correct implementation; proportion of context associated with changed or referenced code.

```text
context_efficiency = useful_context_tokens / total_context_tokens
```

Use this comparatively, not as an absolute truth.

---

## 17. Model Performance Analytics

Dimensions: plan; implement; review; test generation; documentation; UI/UX; architecture; debugging; repository exploration.

Metrics: success rate; first-pass success; reviewer rejection rate; user re-steering; token use; elapsed time; tool-error frequency; rework; CI failure rate; cost.

```text
                    Success   Rework   Tokens   Time
Qwen implementation   93%       7%       low     4m
Codex implementation  97%       3%       high    3m
Qwen planning         76%      31%
Codex planning        96%       5%
```

---

## 18. Adaptive Routing Recommendations

```text
Java refactoring:      Prefer local Qwen.
React UX:              Prefer vision-capable strong model.
Go concurrency:        Prefer Codex.
Documentation:         Prefer local model.
Architecture planning: Prefer stronger reasoning model.
```

Initial implementation SHOULD be recommendation-only. Future AutoSpec routing MAY consume approved routing policies.

---

## 19. Code-Quality Feedback Loop

Sources: complexity scanners; static analysis; linters; type checking; duplication detection; architecture rules; API compatibility checks; tests; mutation tests; security scanners; reviewer feedback; UI automation; visual review.

```text
Implementation -> Quality Gate Failure -> Structured Finding
 -> Session Correlation -> Recurring Pattern -> Improvement Proposal
```

Examples: excessive getters/setters; needless factories; duplicate wrappers; over-abstraction; giant functions; deep nesting; unused interfaces; unneeded feature additions; breaking architecture boundaries; fabricated APIs; excessive mocks.

---

## 20. Improvement Proposal Engine

Proposal types:

```text
agent_instruction, skill, prompt, tool_wrapper, tool_removal, quality_gate,
routing_policy, context_policy, architecture_rule, documentation_update,
test_policy, workflow_change, code_change
```

---

## 21. Proposal Schema

```json
{
  "proposal_id": "proposal_001",
  "finding_id": "finding_001",
  "type": "skill",
  "title": "Add architecture-minimalism skill",
  "status": "draft",
  "rationale": "...",
  "evidence": [],
  "expected_effect": { "rework_reduction_pct": 20, "token_reduction_pct": 10 },
  "risks": [],
  "affected_components": [],
  "patch": "...",
  "evaluation_plan": {},
  "created_by_model": "...",
  "review_model": null
}
```

---

## 22. Evaluation Gate

Every proposal MUST define measurable expected effects.

```text
tool_errors       -20%
tokens/task       -10%
user corrections  -25%
review rework     -15%
success rate       +5%
```

A proposal SHOULD NOT be accepted merely because its wording appears reasonable.

---

## 23. Pre-Merge Evaluation

AutoSpec SHOULD support: historical replay; synthetic benchmark tasks; A/B agent evaluation; shadow mode; static validation; prompt regression tests; skill invocation tests; routing simulation.

Run baseline config vs candidate config against a representative task suite.

---

## 24. Pull Request Workflow

```text
Finding -> Proposal -> Generated patch -> Generated tests -> Evaluation results
 -> Pull Request -> Independent review model -> CI -> Human/project policy approval
```

Self-improvement PRs MUST be clearly labeled: `autospec-improvement`, `agent-policy`, `model-routing`, `skill-change`, `tool-change`, `context-optimization`.

---

## 25. Separation of Duties

The model that creates a self-improvement proposal SHOULD NOT be the sole reviewer.

```text
local analyzer -> strong proposal model -> different strong reviewer
```

AutoSpec's existing role separation policy SHOULD apply.

---

## 26. Post-Deployment Verification

```text
Before:                       After:
tool errors       12.8%       tool errors        8.1%
tokens/task       134k        tokens/task       109k
user corrections   2.3        user corrections   1.3
success rate       89%        success rate       94%
```

Outcomes: `validated`, `inconclusive`, `regressed`. Regressed changes SHOULD trigger a revert/retire recommendation.

---

## 27. Dashboard Integration

The existing AutoSpec / InferWeave CI dashboard SHOULD gain an **Agent Intelligence** area:

```text
Overview, Builds, Pull Requests, Reviews, Agent Intelligence, Models,
Tools & Skills, Context, Quality, Improvement Proposals
```

### 28. Agent Performance Dashboard

Display: task success rate; first-pass success; rework rate; user interventions; turns per task; tokens per task; duration per task; model cost; CI failures; review failures.

Filters: repository; branch; time range; model; provider; role; task type; work item.

### 29. Models Dashboard

Display: model by task type; role performance; success rate; cost; tokens; latency; user corrections; reviewer corrections; fallback rate; routing recommendations.

### 30. Tools & Skills Dashboard

Display: invocation count; success rate; error rate; estimated value; context overhead; unused capability candidates; tool-specific failure patterns.

### 31. Context Dashboard

Display: context size distribution; context at first implementation; files read; files changed; unused reads; repeated reads; context compactions; context exhaustion; estimated context efficiency; learned context graph.

### 32. Quality Dashboard

Display: recurring review findings; complexity regressions; architecture violations; test failures; hallucinated references; security findings; style regressions; user corrections.

### 33. Improvement Dashboard

```text
Active Findings, Proposed Improvements, Under Evaluation, Open PRs,
Monitoring, Validated Improvements, Rejected Improvements, Regressions
```

Every item SHOULD allow drilling into supporting evidence.

---

## 34. Storage Architecture

- PostgreSQL for normalized events, findings, proposals, and metrics.
- Object storage/filesystem for raw session artifacts.
- Optional columnar storage later for high-volume analytics.
- Vector search MAY be used for semantic clustering, but MUST NOT be required for deterministic telemetry.

Core tables:

```text
sessions, session_events, session_summaries, user_interventions,
tool_invocations, model_invocations, git_events, ci_events,
review_findings, quality_findings, patterns, finding_evidence,
improvement_proposals, proposal_evaluations, configuration_versions,
post_change_measurements
```

---

## 35. Evidence Preservation

Every finding MUST be traceable back to evidence: session; event; message; tool call; git commit; PR; CI run; review; quality-gate result.

Do not store only generated summaries.

---

## 36. Semantic Analysis Pipeline

```text
Stage 1 Deterministic extraction
Stage 2 Local session classification
Stage 3 Local failure / intervention extraction
Stage 4 Embedding or semantic grouping
Stage 5 Pattern aggregation
Stage 6 Strong-model diagnosis
Stage 7 Proposal generation
Stage 8 Independent proposal review
```

---

## 37. InferWeave Integration

Bulk enrichment SHOULD run as low-priority InferWeave jobs. Requirements: interactive sessions always take precedence; background analytics can be paused; jobs should be resumable; workloads can fan out to idle nodes; data locality SHOULD be considered; model selection SHOULD prefer inexpensive/local models.

This provides productive work for otherwise idle inference capacity without degrading active coding sessions.

---

## 38. Scheduling

```text
On session completion: deterministic extraction, session summary
Every hour:            incremental enrichment
Daily:                 pattern detection, model/tool/context analytics
Weekly:                strong-model improvement analysis, stale skill/tool review,
                       routing recommendations, improvement report
```

All schedules SHOULD be configurable.

---

## 39. Privacy and Security

The engine MUST support: repository allowlists; secret redaction; configurable retention; local-only semantic analysis; exclusion of sensitive repositories; evidence access controls; audit logs.

Raw prompts MAY contain secrets. Secret scanning SHOULD occur before raw content is sent to remote models. Remote semantic enrichment MUST respect repository/model policy.

---

## 40. Prompt / Skill Growth Control

Every proposed instruction or skill SHOULD consider: does an existing rule already cover this? can this replace another rule? does this apply globally or only to one repository? should this be code/tooling instead of prompt text? does it create contradictions? what context overhead does it add?

Policy priority:

```text
deterministic enforcement > tool validation > quality gate
 > repo-local skill > global skill > AGENTS.md rule
```

Use prompts only where deterministic enforcement is inappropriate.

---

## 41. Conflict Detection

Before accepting prompt/skill changes, AutoSpec SHOULD detect conflicts across AGENTS.md; repo instructions; global instructions; skills; tool descriptions; routing rules.

```text
Rule A: Always use helper X.
Rule B: Never use helper X for service code.
```

Conflicts MUST be surfaced during proposal evaluation.

---

## 42. Tool Wrapper Learning

```text
shell command -> syntax validator -> policy validator -> execute
```

Candidates: ShellCheck; PSScriptAnalyzer; SQL validators; formatter checks; schema validation; API spec validation; infrastructure plan checks.

The improvement engine SHOULD recommend wrappers when failures repeatedly occur before execution.

---

## 43. API Surface

```text
GET  /api/intelligence/sessions
GET  /api/intelligence/sessions/{id}
GET  /api/intelligence/findings
GET  /api/intelligence/findings/{id}
POST /api/intelligence/findings/{id}/dismiss
GET  /api/intelligence/proposals
GET  /api/intelligence/proposals/{id}
POST /api/intelligence/proposals/{id}/evaluate
POST /api/intelligence/proposals/{id}/create-pr
GET  /api/intelligence/models
GET  /api/intelligence/tools
GET  /api/intelligence/context
GET  /api/intelligence/quality
```

---

## 44. CLI

```bash
autospec insights ingest
autospec insights sessions
autospec insights analyze
autospec insights findings
autospec insights finding <id>
autospec insights models
autospec insights tools
autospec insights context
autospec insights propose <finding>
autospec insights evaluate <proposal>
autospec insights create-pr <proposal>
autospec insights report
```

---

## 45. Configuration

```yaml
insights:
  enabled: true

  ingestion:
    pi: true
    codex: false
    claude: false

  semantic_analysis:
    provider: inferweave
    model: local-qwen
    remote_allowed: false

  strong_analysis:
    provider: autospec-router
    role: planning

  retention:
    raw_sessions_days: 90
    normalized_events_days: 365

  thresholds:
    recurring_pattern_min_sessions: 3
    recurring_pattern_min_occurrences: 5
    proposal_confidence_min: 0.80

  self_improvement:
    allow_auto_pr: true
    allow_auto_merge: false

  privacy:
    redact_secrets: true
```

---

## 46. Example End-to-End Flow

1. Agent completes implementation session.
2. Session ingestion records: 93 tool calls; 18 files read; 6 files modified; 4 failed shell commands; user said "use the existing service".
3. Local model classifies intervention: `architecture / reuse_existing_component`.
4. Pattern detector discovers the same correction across 9 sessions.
5. Correlation shows +22% rework, +14% tokens, mostly local implementation model.
6. Strong model reviews aggregated evidence.
7. Proposal: add repo-local "prefer existing architecture" skill and require symbol search before creating service abstractions.
8. Evaluation runs old vs candidate policy.
9. Candidate: -17% tokens, -38% architecture corrections, no success-rate regression.
10. AutoSpec opens PR.
11. Independent reviewer approves.
12. Change merges.
13. Engine monitors next 30 relevant sessions.
14. Improvement marked validated.

---

## 47. Implementation Phases

**Phase 1 — Session Telemetry Foundation.** Pi session ingestion; common event schema; PostgreSQL schema; deterministic session summaries; git correlation; CI correlation; basic CLI; dashboard session view.
Acceptance: >=95% of Pi session events ingest without failure; incremental ingestion is idempotent; sessions link to commits/PRs when identifiers are available; deterministic metrics require no LLM.

**Phase 2 — Semantic Session Enrichment.** Task classification; outcome classification; user re-steering extraction; failure categorization; local InferWeave enrichment.
Acceptance: semantic results retain source evidence; remote models are not required; failed enrichment does not block deterministic telemetry.

**Phase 3 — Pattern Detection.** Recurring user correction detection; tool-error clustering; review finding clustering; stale/unused tool detection; time weighting; finding lifecycle.
Acceptance: patterns require multiple evidence points; every finding exposes supporting sessions; resolved patterns can transition out of active state.

**Phase 4 — Tool / Model / Context Analytics.** Tool ROI; skill ROI; model-by-task analytics; context waste metrics; context graph prototype.
Acceptance: dashboards filter by model/repo/task; unused capability candidates are generated; model routing recommendations are evidence-backed.

**Phase 5 — Improvement Proposal Engine.** Proposal schema; remediation generation; prompt/skill conflict detection; evaluation plan generation; proposal UI.
Acceptance: no proposal can modify production automatically; every proposal links to findings/evidence; every proposal includes expected measurable impact.

**Phase 6 — Evaluation and PR Generation.** Benchmark/replay harness; candidate vs baseline evaluation; PR generation; independent review integration; self-improvement labels.
Acceptance: candidate changes produce evaluation reports; auto-merge remains impossible; review model differs from proposal generator where available.

**Phase 7 — Closed-Loop Measurement.** Configuration version tracking; pre/post comparison; improvement validation; regression detection; revert/retire proposals.
Acceptance: every merged self-improvement can be tied to measurable before/after metrics; regressed changes surface automatically.

---

## 48. Testing Strategy

**Unit tests** cover: Pi parser; event normalization; metric calculations; idempotent ingestion; finding thresholds; lifecycle transitions; proposal validation; conflict detection.

**Integration tests** cover: Pi session -> database; database -> summary; summary -> semantic enrichment; findings -> proposal; proposal -> evaluation; proposal -> PR metadata.

**Golden session fixtures** containing: successful autonomous work; repeated user corrections; excessive repository exploration; context exhaustion; broken shell commands; over-abstraction; unused skill invocation; model fallback; reviewer rejection; CI failure.

**Regression tests** maintain expected findings for fixtures. A change to the analyzer MUST NOT silently alter historical classification without an intentional fixture update.

---

## 49. Observability

```text
autospec_insights_sessions_ingested_total
autospec_insights_events_ingested_total
autospec_insights_ingestion_errors_total
autospec_insights_enrichment_jobs_total
autospec_insights_enrichment_failures_total
autospec_insights_findings_active
autospec_insights_proposals_active
autospec_insights_proposals_validated_total
autospec_insights_proposals_regressed_total
autospec_insights_analysis_queue_depth
autospec_insights_analysis_gpu_seconds
```

Structured logs MUST include `session_id`, `finding_id`, `proposal_id`, `repo`, `model`, and `work_item_id` where relevant.

---

## 50. Failure Handling

The subsystem MUST degrade gracefully: semantic model unavailable -> deterministic analytics continue; InferWeave node unavailable -> queue enrichment; corrupted session -> quarantine session and continue; git correlation unavailable -> retain unmatched session; dashboard unavailable -> ingestion continues; proposal generation fails -> finding remains active.

Historical analytics MUST NOT interfere with active AutoSpec coding tasks.

---

## 51. Performance Requirements

- ingest at least 1M normalized events/day on modest infrastructure;
- incremental processing rather than repeated full-history scans;
- bulk local inference jobs parallelizable across InferWeave nodes;
- dashboard aggregate queries target <2s for common views;
- raw history MUST NOT be reprocessed unless analyzer version changes or explicitly requested.

---

## 52. Versioning

Version the event schema; summary schema; semantic classifier; finding detector; proposal generator; configuration.

```text
extractor_version: 1.2.0
classifier_version: 0.4.1
pattern_engine_version: 0.3.0
```

Required for reproducibility.

---

## 53. Migration Strategy

Start as an additive subsystem. Do NOT initially change AutoSpec routing or agent policy automatically.

```text
observe -> analyze -> recommend -> evaluate -> PR -> optionally consume approved policies
```

---

## 54. Success Metrics

Reduces: unnecessary tool calls; repeated user corrections; reviewer rework; tool errors; context size; context compactions; CI failures; repeated architectural violations; task duration; expensive-model usage where unnecessary.

Improves: first-pass success; autonomous completion; test success; reviewer acceptance; routing accuracy; skill/tool utilization; useful work per token.

---

## 55. Initial Recommended MVP Scope

1. Pi JSONL ingestion.
2. Deterministic metrics.
3. User correction classification.
4. Tool errors.
5. Files/context usage.
6. Git/PR/CI outcome linkage.
7. Finding generation.
8. Basic Agent Intelligence dashboard.
9. Local InferWeave semantic enrichment.
10. Human-reviewed improvement proposals.

Do NOT start with automated self-change.

---

## 56. Recommended Repository Structure

```text
internal/insights/{ingest/{pi,autospec,ci},events,summarize,enrich,patterns,
                   models,tools,context,quality,proposals,evaluation,storage}
web/agent-intelligence/{sessions,findings,models,tools,context,proposals}
cmd/autospec/insights
```

Adapt structure to existing AutoSpec architecture rather than forcing these exact paths.

---

## 57. Critical Acceptance Criteria

The feature is NOT complete unless all of the following are true:

- Pi sessions can be ingested incrementally.
- Deterministic telemetry works without an LLM.
- Raw evidence remains traceable.
- User re-steering is captured.
- Repeated patterns can be detected.
- Model performance can be compared by task class.
- Tool/skill use can be measured.
- Context waste can be measured.
- Findings appear in the CI dashboard.
- Findings can generate improvement proposals.
- Proposals include measurable expected outcomes.
- Candidate changes can be evaluated against baseline.
- Self-improvement changes require PR/review.
- Auto-merge of self-improvement is impossible by default.
- Post-change impact is measured.
- Regressions can be detected.
- Historical analytics never block interactive AutoSpec execution.

---

## 58. Future Extensions

Automated benchmark generation from historical failures; repository-specific agent curricula; learned context routing; learned tool selection; dynamic routing policies; session outcome prediction; anomaly detection; cross-repository architecture insight; prompt/skill minimization; automated retirement experiments; organization-wide model benchmarking; learned Scout policies; codebase-specific architectural knowledge graphs; reinforcement-style routing optimization.

These SHOULD be separate follow-up specs after the telemetry and evidence foundation is stable.

---

## 59. Final Product Principle

AutoSpec should not merely become better at generating code. It should become better at engineering its own agent system.

```text
Plan -> Implement -> Review -> Measure -> Learn -> Propose -> Evaluate
 -> Improve -> Measure Again
```

Every optimization must remain evidence-backed, testable, auditable, and reversible.
