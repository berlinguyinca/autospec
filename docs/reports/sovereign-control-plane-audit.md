# Sovereign control plane Phase 5.5 audit

Issue: #1622 — Phase 5.5 audit + remediation — autospec sovereign control plane  
Source spec: `docs/specs/2026-07-08-autospec-sovereign-control-plane-design.md`  
Audit date: 2026-07-09

## Scope

This audit compares the merged control-plane child issues (#1611–#1621) against the MVP acceptance criteria in the source spec. The audit focused on integration, privacy, validation, and release-blocking drift. It did not add new MVP features beyond remediating validation coverage drift.

## MVP acceptance item matrix

| Spec MVP acceptance item | Evidence checked | Status | Decision |
| --- | --- | --- | --- |
| `autospec-control-plane bootstrap --dry-run` prints the two repo scaffolds without creating GitHub repos. | `tests/control-plane-bootstrap.bats`; `tests/control-plane-observatory.bats` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| `autospec-control-plane bootstrap --confirm` creates or updates `autospec-governance` and `autospec-observatory`. | `tests/control-plane-confirm.bats` | Pass after remediation | Test existed but was not in `scripts/validate.sh`; added to validation gate. |
| `autospec-governance` contains policy/rule YAML, schemas, fixtures, docs, and a test command that passes. | `tests/control-plane-governance.bats` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| `autospec-observatory` starts locally with Postgres and a web UI. | `tests/control-plane-observatory.bats`; generated `docker-compose.yml`; generated `apps/web` scaffold | Partial MVP scaffold | Dry-run scaffold is covered. Live Docker startup remains operator/manual evidence because the generated companion repo is not materialized in this repo. |
| `POST /v1/events/batch` accepts scoped API-key authenticated event batches. | `tests/control-plane-events.bats`; `tests/control-plane-observatory-auth.bats`; `tests/integration/control-plane-events.bats` | Pass after remediation | Integration wrapper existed but was not in `scripts/validate.sh`; added to validation gate. |
| The observatory stores runs, events, workers, projects, repositories, and costs in Postgres. | `tests/control-plane-observatory-auth.bats`; `tests/control-plane-reports.bats` | Pass | Migrations and report fields are generated and validated. |
| The web UI shows fleet, timeline, work item, blockers, workers, policy decision, and cost/duration/outcome views with 10-second polling. | `tests/control-plane-ui.bats`; `tests/smoke/control-plane-ui.bats`; `tests/control-plane-reports.bats` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| The web UI shows a per-run progress bar and progress detail panel with current item, queue counts, elapsed time, ETA, planned next step, and stale/error state. | `tests/control-plane-ui.bats`; `tests/smoke/control-plane-ui.bats` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| `GET /v1/runs/:id/progress` returns the latest progress snapshot and updates from `ProgressUpdated` plus existing run/work-item events. | `tests/control-plane-observatory-auth.bats`; `tests/control-plane-events.bats` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| Autospec emits structured events to a local outbox and flushes them when configured. | `tests/observatory-outbox.bats`; `docs/runbooks/OBSERVATORY.md` | Pass | Covered by existing tests and `scripts/validate.sh`. |
| Autospec continues working when the observatory is offline. | `AUTOSPEC_OBSERVATORY_OFFLINE=1 bash tests/observatory-outbox.bats`; `tests/control-plane-dogfood.bats` | Pass after remediation | Outbox test was covered; dogfood offline replay existed but was not in `scripts/validate.sh`; added to validation gate. |
| Privacy-tier enforcement rejects over-shared events both client-side and server-side. | `tests/policy-resolution.bats`; generated auth/API-key tier contracts | Pass after remediation | Test existed but was not in `scripts/validate.sh`; added to validation gate. Server-side generated scaffold remains a contract-level dry-run, not a deployed service proof. |
| Project classification is visible and filterable in the observatory UI. | `tests/control-plane-governance.bats`; `tests/control-plane-reports.bats` | Pass | Covered by generated policy packs and report filters. |
| A dogfood run against `berlinguyinca/autospec` produces a run timeline and cost report in the observatory. | `tests/control-plane-dogfood.bats`; `docs/runbooks/CONTROL_PLANE_DOGFOOD.md` | Pass after remediation | Dogfood smoke existed but was not in `scripts/validate.sh`; added to validation gate. |

## Child issue validation coverage

| Issue | Primary smoke | Validation gate status |
| --- | --- | --- |
| #1611 | `bash tests/control-plane-bootstrap.bats` | Covered before audit. |
| #1612 | `bash tests/control-plane-governance.bats` | Covered before audit. |
| #1613 | `bash tests/control-plane-observatory.bats` | Covered before audit. |
| #1614 | `bash tests/control-plane-observatory-auth.bats` | Covered before audit. |
| #1615 | `bash tests/control-plane-events.bats` plus integration wrapper | Integration wrapper added to `scripts/validate.sh`. |
| #1616 | `bash tests/control-plane-ui.bats` plus smoke wrapper | Covered before audit. |
| #1617 | `bash tests/control-plane-reports.bats` plus integration wrapper | Covered before audit. |
| #1618 | `AUTOSPEC_OBSERVATORY_OFFLINE=1 bash tests/observatory-outbox.bats` | Covered before audit. |
| #1619 | `bash tests/policy-resolution.bats` | Added to `scripts/validate.sh`. |
| #1620 | `bash tests/control-plane-confirm.bats` | Added to `scripts/validate.sh`. |
| #1621 | `bash tests/control-plane-dogfood.bats` | Added to `scripts/validate.sh`. |

## Remediation performed

- `scripts/validate.sh` now requires and runs the missing control-plane policy/privacy, confirm-bootstrap, dogfood, and event-integration Bats suites.
- No generated companion-repo runtime implementation was broadened in this audit.

## Follow-up decisions

No new follow-up issue was filed from this audit. The only concrete release-blocking drift found was small and in scope: missing validation enumeration for already-merged control-plane tests. The remaining partial-live-service proof is accepted as MVP scaffold evidence because the child issues implemented dry-run/generated companion repository contracts rather than materializing and deploying `autospec-observatory` inside this repository.

## Verification evidence

Re-run these commands from the PR branch:

```bash
bash tests/control-plane-bootstrap.bats
bash tests/control-plane-governance.bats
bash tests/control-plane-observatory.bats
bash tests/control-plane-observatory-auth.bats
bash tests/control-plane-events.bats
bash tests/integration/control-plane-events.bats
bash tests/control-plane-ui.bats
bash tests/smoke/control-plane-ui.bats
bash tests/control-plane-reports.bats
bash tests/integration/control-plane-reports.bats
AUTOSPEC_OBSERVATORY_OFFLINE=1 bash tests/observatory-outbox.bats
bash tests/policy-resolution.bats
bash tests/control-plane-confirm.bats
bash tests/control-plane-dogfood.bats
bash scripts/validate.sh
```
