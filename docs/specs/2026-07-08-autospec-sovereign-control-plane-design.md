# Autospec Sovereign Control Plane

**Date:** 2026-07-08
**Status:** draft for autospec decomposition
**Builds on:** `docs/specs/2026-07-06-autospec-autonomous-platform-design.md`, `docs/specs/2026-06-27-constitution-baseline-integration.md`, `docs/specs/AUTOSPEC_CONSTITUTION_MASTER_SPEC.md`
**Companion spec:** `docs/specs/2026-07-08-autospec-sovereign-control-plane-future-versions.md`

## Goal

Turn autospec from a repo-local autonomous executor into a fleet-aware autonomous
engineering system with a public governance brain, a SaaS-ready observability and
reporting service, and a stable autospec integration contract that records what
work was done, why it was selected, who or what performed it, how long it took,
what it cost, and whether the outcome improved the project.

The system has three parts:

1. `autospec-governance` — a public, versioned, tested policy and rule catalog.
2. `autospec-observatory` — a SaaS-ready network service for developer/operator analytics.
3. `autospec` integration — policy resolution, event emission, local outbox, privacy enforcement, and repo bootstrap.

## Non-goals

- Do not replace GitHub labels as the live autonomous control channel in MVP.
- Do not implement billing or payments in MVP, even though billing boundaries are modeled.
- Do not ingest raw logs by default.
- Do not require WebSockets or server-sent events; 10-second polling is enough for MVP.
- Do not allow arbitrary executable policy scripts from `autospec-governance`.
- Do not make `autospec` itself the database, dashboard, or policy catalog.

## Repository Split

### `autospec-governance`

Public repository containing reusable, versioned, tested policy packs and rule
catalogs. It defines what autonomous autospec should prioritize and what evidence
is required by project class.

Initial layout:

```text
autospec-governance/
  policies/
    open-source-maintainer-default.yml
    private-personal-default.yml
    private-company-default.yml
    client-project-default.yml
    research-default.yml
    sandbox-default.yml
  rules/
    qa.yml
    testing.yml
    documentation.yml
    security.yml
    accessibility.yml
    performance.yml
    skill-generation.yml
    release-readiness.yml
  schemas/
    policy.schema.json
    rule.schema.json
    project-class.schema.json
    priority.schema.json
  fixtures/
    projects/
      open-source-cli.yml
      private-saas.yml
      client-webapp.yml
      ai-product.yml
  tests/
    policy-schema.bats
    priority-resolution.bats
    privacy-tier.bats
    merge-rules.bats
    project-classification.bats
    cost-limits.bats
    evidence-requirements.bats
  docs/
    policy-authoring.md
    project-classes.md
    priority-waterfall.md
```

Policies are data, not code. Validation is deterministic: JSON Schema plus fixture
tests. Autospec may cache policies locally and must record the policy ID, version,
digest, and resolution trace used for each run.

### `autospec-observatory`

SaaS-ready network service for analytics and reporting across many autospec workers,
repositories, companies, project classes, and operators.

Initial service layout:

```text
autospec-observatory/
  apps/
    api/
    web/
  packages/
    event-schema/
    db/
    ui/
  migrations/
  docs/
  docker-compose.yml
```

The MVP service includes:

- Postgres-backed multi-tenant data model.
- API-key authentication.
- Event ingestion API.
- Local outbox replay/dedupe support.
- Developer/operator web UI.
- Cost, duration, outcome, worker, blocker, and timeline reporting.
- Project classification and privacy-tier enforcement.

### `autospec`

Autospec remains the executor. It gains:

- companion-repo bootstrap command;
- governance policy resolver;
- observatory event emitter;
- local durable outbox;
- project classification resolver;
- privacy-tier scrubber;
- run/cycle/work-item event schema;
- dogfood command proving end-to-end flow across generated repos.

## Project Classification

Every project has exactly one primary classification:

```text
open-source
private-personal
private-company
client-project
research
sandbox
```

Classification is stored in the observatory project registry and may be mirrored
in repo-local `.autospec/autonomous.yml`. It affects default policy, priority order,
privacy tier, evidence expectations, merge rules, cost ceilings, and reporting.

Examples:

- `open-source`: prioritize security, CI health, tests, docs, release hygiene,
  accessibility, public examples, license/IP checks, and minimal private data capture.
- `private-personal`: prioritize velocity, learning, broad exploration, and bounded
  cost controls.
- `private-company`: prioritize QA, business workflows, deployment health, internal
  documentation, security, and cost controls.
- `client-project`: prioritize audit trail, scope boundaries, cost attribution,
  report exports, and stronger privacy defaults.
- `research`: prioritize reproducibility, notebooks/data safety, provenance, and
  result documentation.
- `sandbox`: allow broader debug capture and experimentation with explicit limits.

## Privacy Tiers

Observatory ingestion is privacy-tiered:

```text
metadata-only
summary
evidence
full-debug
```

Default tiers by project class:

| Project class | Default tier | Raw logs allowed by default |
| --- | --- | --- |
| `open-source` | `summary` | no |
| `private-personal` | `evidence` | no |
| `private-company` | `summary` | no |
| `client-project` | `metadata-only` | no |
| `research` | `summary` | no |
| `sandbox` | `evidence` | yes, if API key allows it |

Autospec must enforce the privacy tier before sending data. The observatory must
also reject events that exceed the API key's maximum privacy tier.

## API-Key Authentication

MVP auth uses scoped API keys. Each key has:

```text
key_id
name
owner_org_id
allowed_project_ids[]
allowed_repo_patterns[]
allowed_event_scopes[]
privacy_tier_limit
created_by
created_at
last_seen_at
revoked_at
```

Scopes:

```text
events:write
events:read
projects:read
projects:write
runs:read
costs:read
admin:keys
```

Rules:

- API keys are stored hashed and shown once at creation.
- Every key belongs to one org.
- Every event belongs to one org.
- A key can write only to allowed projects or repo patterns.
- A key cannot exceed its privacy tier.
- Revoked keys fail immediately.
- Auth failures are written as security events.

## Tenant Model

The observatory data hierarchy is:

```text
platform
  org
    workspace
      project
        repository
          run
            cycle
              work_item
                event
```

Core entities:

- `orgs`: companies, personal accounts, or open-source orgs.
- `workspaces`: teams, client portfolios, or departments inside an org.
- `projects`: product/project containers.
- `repositories`: GitHub repositories attached to a project.
- `operators`: humans or service accounts.
- `workers`: machines/processes running autospec.
- `agents`: logical Codex, Claude, OpenCode, or future agents.
- `runs`: autonomous or manual autospec invocations.
- `events`: append-only operational facts.

Cross-org reads are forbidden by default. Open-source projects may expose public
summary reports only when explicitly configured.

## Event Protocol

Autospec emits structured events, not arbitrary logs by default.

Initial event types:

```text
RunStarted
RunStopped
RunPaused
CycleStarted
CycleCompleted
WorkItemSelected
IssueClassified
ImplementationStarted
PRCreated
ValidationStarted
ValidationPassed
ValidationFailed
PRMerged
WorkItemBlocked
CostRecorded
OperatorIntervened
PolicyResolved
WorkerHeartbeat
ProgressUpdated
```

Every event includes:

```text
event_id
run_id
sequence
occurred_at
received_at
org_id
workspace_id
project_id
repository_id
project_classification
privacy_tier
operator_id
worker_id
agent_id
harness
model
skill_or_workflow
issue_url
pr_url
commit_sha
policy_id
policy_version
policy_digest
duration_ms
estimated_cost_usd
actual_cost_usd
risk_level
status
summary
progress_percent
progress_phase
current_item_title
current_item_url
queue_ready_count
queue_blocked_count
queue_claimed_count
queue_remaining_count
estimated_next_item_at
estimated_completion_at
planned_next_step
artifact_links[]
```

Progress fields are normalized operator telemetry, not decoration. Autospec should
emit `ProgressUpdated` whenever the conductor selects work, starts implementation,
opens a PR, starts validation, blocks, parks, merges, or recalculates queue state.
When exact completion is unknowable, the emitter should still provide bounded,
human-readable progress: current phase, current work item, queue counts, planned
next step, and a best-effort ETA with confidence.

Ordering:

- Worker assigns monotonically increasing `sequence` per run.
- Duplicate `event_id` is ignored.
- Repeated sequence is stored once and flagged.
- Sequence gaps are visible in the UI.
- Late events are stored and exposed by both `occurred_at` and `received_at`.

## Local Outbox

Autospec never blocks implementation on observatory availability. Events are written
to a durable local outbox before upload:

```text
.autospec/observatory/outbox/<run-id>.jsonl
.autospec/observatory/checkpoints.json
```

Rules:

- Flush every N seconds during long runs and at run end.
- Retry with exponential backoff.
- Preserve event order per run.
- Dedupe by `event_id`.
- Continue working if the observatory is offline.
- Surface upload failures in local status/timeline reports.

## Network API

MVP API:

```http
POST /v1/events
POST /v1/events/batch
GET  /v1/projects
GET  /v1/projects/resolve?repo=OWNER/REPO
GET  /v1/projects/:id/runs
GET  /v1/runs/:id/progress
GET  /v1/runs/:id/timeline
GET  /v1/runs/:id/events?after_event_id=...
GET  /v1/runs/:id/costs
GET  /v1/fleet/summary
GET  /v1/workers?status=active
GET  /v1/policies/effective?project_id=...
POST /v1/api-keys
DELETE /v1/api-keys/:id
```

Polling is the v1 update model. UI pages refresh every 10 seconds by default,
with 5s, 10s, 30s, and paused options. Responses include:

```json
{
  "events": [],
  "next_cursor": "evt_...",
  "server_time": "2026-07-08T00:00:00Z",
  "poll_after_ms": 10000
}
```

`GET /v1/runs/:id/progress` returns the latest normalized progress snapshot:

```json
{
  "run_id": "run_...",
  "status": "running",
  "progress_percent": 42,
  "phase": "implementation",
  "current_item": {
    "title": "Build observatory operator UI shell",
    "url": "https://github.com/..."
  },
  "queue": {
    "ready": 3,
    "claimed": 1,
    "blocked": 11,
    "remaining": 14
  },
  "elapsed_ms": 5400000,
  "current_item_elapsed_ms": 1200000,
  "estimated_next_item_at": "2026-07-08T20:20:00Z",
  "estimated_completion_at": "2026-07-08T23:30:00Z",
  "eta_confidence": "low",
  "planned_next_step": "wait for PR checks, merge if green, then pick #1606",
  "last_event_id": "evt_..."
}
```

## Developer/Operator UI

The UI is dense, utilitarian, and operator-focused. It should feel like GitHub
Actions, Sentry, and Datadog for autonomous engineering, not a marketing or
executive BI dashboard.

Primary screens:

1. **Live Fleet** — projects, repos, status, current item, agent, model, elapsed,
   cost, risk, last event, and an inline progress bar for each active run.
2. **Run Timeline** — chronological human-readable activity stream.
3. **Run Progress** — a plain-English progress panel with percent complete,
   current phase, current item, item elapsed time, queue counts, estimated time
   until the next item starts, estimated completion, planned next step, stale
   heartbeat warnings, and the last event that changed the estimate.
4. **Work Item Detail** — issue, policy decision, branch/worktree, PR, commits,
   validations, CI, cost, duration, blocker, artifacts.
5. **Queue/Backlog** — `auto-implement`, `needs-classify`,
   `needs-autospec-template`, blocked issues, priority order.
6. **Failures/Blockers** — failed CI, failed validation, secret scan, stale lock,
   malformed issue, missing policy, main red, quota park.
7. **Workers/Agents** — host, repo, PID/session, cycle, heartbeat, lock owner,
   model/harness, last log line, status.
8. **Policy Decision Inspector** — resolved policy, policy digest, project class,
   priority score, risk score, privacy tier, merge permission, rejected alternatives.
9. **Cost/Duration/Outcome Reports** — per org, workspace, project, repo, operator,
   worker, model, skill, issue, PR, and time window.

The progress bar must be useful even when autospec cannot know the true total
amount of future work. For backlog-drain runs, compute percent from completed
work items over completed + ready + claimed + blocked known work. For never-idle
discovery loops, cap progress at the current bounded batch or cycle window and
label the run as continuous. A stale heartbeat, missing PR, blocked validation,
quota park, or human-needed state must be reflected beside the bar instead of
pretending forward progress is happening.

Filters:

```text
date range
project classification
org/company
workspace
project
repo
operator
worker
agent/harness/model
skill/workflow
policy version
privacy tier
risk level
status/outcome
cost range
duration range
```

## Cost, Duration, and Outcome Reporting

The observatory must support:

- cost per org, workspace, project, repository, model, harness, skill, operator,
  worker, issue, PR, run, and date range;
- duration from issue selected to PR opened, PR opened to CI green, CI green to
  merge, validation time, blocked time, review time, rework time, and total cycle time;
- outcomes: merged, failed, blocked, reverted, needs human, generated follow-up,
  validation passed/failed, downstream regression.

MVP reports:

```text
Project weekly summary
Client billing export
Open-source maintenance report
Agent performance report
Cost anomaly report
Blocked work report
Autonomous ROI report
```

Billing and payments are future work; billable dimensions are still modeled in MVP.

## Companion Repo Bootstrap

Autospec must be able to generate the two companion repos:

```bash
autospec-control-plane bootstrap \
  --owner berlinguyinca \
  --governance-repo autospec-governance \
  --observatory-repo autospec-observatory
```

The command:

1. creates `autospec-governance` if absent;
2. scaffolds policies, rules, schemas, fixtures, tests, and docs;
3. creates `autospec-observatory` if absent;
4. scaffolds API, web UI, migrations, compose file, docs, and seed data;
5. commits each repo with conventional commits;
6. pushes both repos to GitHub;
7. writes `.autospec/control-plane.json` in the autospec repo with repo URLs and
   bootstrap metadata;
8. emits `ControlPlaneBootstrapStarted`, `GovernanceRepoCreated`,
   `ObservatoryRepoCreated`, and `ControlPlaneBootstrapCompleted` events when an
   observatory endpoint is configured.

## Autospec Policy Resolution

Autospec resolves policy in this order:

1. repo-local `.autospec/autonomous.yml`;
2. observatory project assignment;
3. `autospec-governance` default for project classification;
4. built-in safe fallback.

Each run emits:

```text
policy_source
policy_id
policy_version
policy_digest
policy_resolution_trace
```

Policy validation must pass before a downloaded governance policy is trusted.

## MVP Acceptance Criteria

- `autospec-control-plane bootstrap --dry-run` prints the two repo scaffolds without
  creating GitHub repos.
- `autospec-control-plane bootstrap --confirm` creates or updates
  `autospec-governance` and `autospec-observatory`.
- `autospec-governance` contains policy/rule YAML, schemas, fixtures, docs, and
  a test command that passes.
- `autospec-observatory` starts locally with Postgres and a web UI.
- `POST /v1/events/batch` accepts scoped API-key authenticated event batches.
- The observatory stores runs, events, workers, projects, repositories, and costs
  in Postgres.
- The web UI shows fleet, timeline, work item, blockers, workers, policy decision,
  and cost/duration/outcome views with 10-second polling.
- The web UI shows a per-run progress bar and progress detail panel with current
  item, queue counts, elapsed time, ETA, planned next step, and stale/error state.
- `GET /v1/runs/:id/progress` returns the latest progress snapshot and updates
  from `ProgressUpdated` plus existing run/work-item events.
- Autospec emits structured events to a local outbox and flushes them when configured.
- Autospec continues working when the observatory is offline.
- Privacy-tier enforcement rejects over-shared events both client-side and server-side.
- Project classification is visible and filterable in the observatory UI.
- A dogfood run against `berlinguyinca/autospec` produces a run timeline and cost
  report in the observatory.

## MVP Issue Decomposition Hints

Autospec should decompose this spec into these implementation lanes:

1. Governance repo scaffold and validator.
2. Governance policy/rule schemas and fixtures.
3. Observatory API, DB schema, migrations, and API-key auth.
4. Observatory event ingestion and dedupe.
5. Observatory developer/operator UI shell.
6. Observatory analytics reports and filters.
7. Autospec event schema and local outbox.
8. Autospec policy resolver and privacy-tier scrubber.
9. Autospec companion-repo bootstrap command.
10. End-to-end dogfood wiring and validation docs.

## Future Versions

Future version specs are tracked in
`docs/specs/2026-07-08-autospec-sovereign-control-plane-future-versions.md`.
They are intentionally outside MVP scope but must shape data-model and API
decisions so v1 does not paint the platform into a corner.
