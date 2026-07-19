# Autospec Sovereign Control Plane Future Versions

**Date:** 2026-07-08
**Status:** roadmap companion spec
**Parent spec:** `docs/specs/2026-07-08-autospec-sovereign-control-plane-design.md`

## Goal

Preserve the long-term control-plane vision while keeping the first implementation
bounded. Each version below is a follow-up spec seed that autospec can later
materialize into a full design and issue package.

## Version V1 — MVP Networked Analytics

Tracked by the parent spec.

Scope:

- generate `autospec-governance`;
- generate `autospec-observatory`;
- ingest structured autospec events;
- expose developer/operator dashboards with per-run progress bars, queue counts,
  current item, ETA, planned next step, and stale/error state;
- support project classification, API keys, privacy tiers, cost/duration/outcome
  reports, and 10-second polling;
- add autospec local outbox, policy resolver, and companion repo bootstrap.

Exit criteria:

- dogfood run against `berlinguyinca/autospec` is visible in observatory;
- dogfood run progress is visible as a live progress bar and detail panel;
- governance policy validation passes;
- observatory API and UI run locally and in a containerized deployment;
- telemetry loss does not block autospec work.

## Version V2 — Policy Intelligence and Governance Marketplace

Scope:

- richer public policy catalog;
- semantic policy diff and changelog;
- policy compatibility checker for target repos;
- policy simulation before applying to a repo;
- reusable project-class overlays;
- public docs site for rule packs;
- governance policy scoring and linting;
- policy deprecation lifecycle.

Acceptance criteria:

- autospec can simulate how a policy would prioritize a repo before enabling it;
- policy changes publish machine-readable changelogs;
- policy packs are searchable and documented;
- policy tests cover priority, privacy, evidence, cost, and merge behavior.

## Version V3 — Advanced Analytics and ROI

Scope:

- ROI scoring by project and work type;
- cost anomaly detection;
- model efficiency comparisons;
- cycle-time trend analysis;
- failure clustering;
- rework and regression tracking;
- quality improvement curves over time;
- report exports for clients and company stakeholders.

Acceptance criteria:

- dashboard identifies expensive low-value work patterns;
- project reports show autonomous value delivered per dollar and per hour;
- recurring blocker classes are grouped automatically;
- exported reports can be shared outside the engineering team without raw logs.

## Version V4 — Operator Control Cockpit

Scope:

- live control surface for pause, resume, stop, priority changes, and steering;
- UI-backed GitHub label operations;
- guarded control actions with audit logs;
- per-project runbook actions;
- approvals for high-risk quarantined work;
- control history timeline.

Acceptance criteria:

- UI can steer autospec without bypassing the GitHub label/control-channel model;
- every operator action is auditable;
- control actions respect project permissions and API-key/user scopes;
- high-risk approvals remain explicit and traceable.

## Version V5 — Multi-Worker Cluster Scheduling

Scope:

- worker pool registry;
- work assignment across 10-50 concurrent autospec processes;
- distributed locks and leases;
- per-project concurrency budgets;
- fairness across orgs/projects;
- worker health and capability matching;
- autoscaling hooks.

Acceptance criteria:

- multiple workers drain independent work without stepping on branches, locks, or
  repo-local state;
- project concurrency limits are enforced;
- stale workers are reclaimed safely;
- scheduler can explain why each worker got each task.

## Version V6 — Artifact and Evidence Vault

Scope:

- object-storage-backed artifact ingestion;
- signed evidence bundles;
- retention policies by project class;
- redaction and scrub pipelines;
- screenshot, trace, log, coverage, report, and validation artifact browsing;
- immutable audit exports.

Acceptance criteria:

- artifacts are stored only when privacy tier and API key permit them;
- client-project exports include signed audit summaries;
- retention policies delete or archive artifacts deterministically;
- raw debug artifacts are never enabled by accident.

## Version V7 — Hosted SaaS Commercial Readiness

Scope:

- subscription and billing integration;
- org onboarding;
- usage quotas;
- invoices and chargeback reports;
- account administration;
- hosted deployment hardening;
- backups, migrations, disaster recovery, and audit logging.

Acceptance criteria:

- every event has a billable org/project association;
- billing can be calculated without changing the event schema;
- org admins can manage users, API keys, projects, and privacy defaults;
- hosted service meets baseline operational requirements.

## Version V8 — Learning and Recommendation Engine

Scope:

- cross-project learning from outcomes;
- recommended governance policy changes;
- agent/model routing recommendations;
- predicted duration and cost before work starts;
- risk prediction;
- automatic follow-up spec suggestions;
- operator-persona integration.

Acceptance criteria:

- recommendations cite historical evidence and confidence;
- operators can accept/reject recommendations;
- accepted recommendations update governance policy or repo-local config through
  normal review paths;
- predictions are measured against actual outcomes.

## Cross-Version Invariants

- Autospec execution must not depend on observatory uptime.
- Public governance data and private execution history remain separate.
- Every autonomous decision must be explainable by policy, project classification,
  observed signals, and run context.
- Privacy-tier enforcement happens before upload and again at ingestion.
- Event schemas are versioned and backwards compatible.
- GitHub remains the source of truth for issues, PRs, commits, and merge state.
- The observatory is analytics/reporting first until V4.
