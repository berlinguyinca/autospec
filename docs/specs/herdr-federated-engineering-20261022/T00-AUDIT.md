# T00 — Source audit and REUSE / EXTEND / CONSOLIDATE / NEW matrix

Program: herdr-federated-engineering-20261022. Audited 2026-10-22 at these commits:

| Repo | SHA |
|---|---|
| pi-engineering | e5bf071 |
| herdr-engineering | 5ec60af |
| autospec | bcf8fd4f |
| autospec-constitution | f7ad0f4 |
| autospec-baselines | 581e096 |
| autospec-design | 167ed87 |

Method: read-only source inspection. No test was executed during the audit; a test is listed only if a file was found
that exercises the behaviour. "Complete / Partial / Absent" describes what the code does, not what the spec's
entry-point list implies. Forbidden targets (InferWeave/inferweave-dashboard, InferWeave/inferweave-workbench) were not touched.

## Corrections to the spec's entry points
- `herdr_engineering/control_plane/reviews.py` is the **Plannotator receipt service** (register/heartbeat/complete/fail,
  bound to instance_id + revision + plan_digest). It has no PR review, head-SHA or quorum logic.
- `herdr_engineering/pi_autospec.py` is **imported by no runtime module** (only its test). The live Pi→Herdr path is
  `mission_activity_feed.py` → `IngestService` → `control_plane/mission_activity.py`.
- `REVIEW_QUORUM.md` is at `autospec/docs/runbooks/REVIEW_QUORUM.md`.
- `autospec/.../review_dispatch.rs` only builds argv for the Phase 5.5 broad review; it is not a post-PR reviewer.
- Pi `src/lifecycle/telemetry.ts` carries no schema_version/event_id/source/sequence/causation/actor/visibility/evidence.

## Matrix

| # | Feature | Verdict | Existing evidence | Gap / work |
|---|---|---|---|---|
| 1 | Event envelope | CONSOLIDATE + EXTEND | Pi: `LedgerEvent` (src/core/types.ts), `PlatformEvent` (src/platform/types.ts), `TelemetryEvent` (src/lifecycle/telemetry.ts), autospec `Envelope` (src/orchestration/externalEntryProtocol.ts), `InspectorEvent` (src/runtime/herdr/InspectorFeed.ts). Herdr: `Event`/`EventStore` (control_plane/events.py), `fabric_events` PK event_id + ON CONFLICT DO NOTHING (deploy/control-plane/migrations/0001_control_plane.sql). | Five Pi shapes, one Herdr shape. Missing everywhere: `source_sequence`, `causation_id`, `visibility_scope`, `evidence_refs`, org/workspace/task/agent ids as columns. One canonical contract; Pi emits it, Herdr ingests it. (T01) |
| 2 | Idempotent replay / dedupe | EXTEND | Herdr `fabric_events` conflict-ignore; Pi `SessionEventStore` + `missionHandover` dedupe by event_id; `EventStore.read_stream`, `CollabService.hydrate`. | Dedupe only; no per-source ordering, gap detection, freshness or reconciliation. (T01) |
| 3 | Lease fencing | REUSE | Pi `src/orchestration/ownership.ts` (fencingToken, "stale mission fencing identity"), `runtime/isolation/{LeaseManager,MissionCustody,leaseReconcile}.ts`; tests mission-ownership, lease-reconcile, mergequeue-sha-fence. | Surface fencing token in envelope/actions only. |
| 4 | Worktree isolation | REUSE | Pi `src/git/GitRepo.ts`, `runtime/isolation/WorktreeIdentity.ts`, `guard/primaryWorktreeGuard.ts`. | None. |
| 5 | Worker roles / spawn | REUSE + EXTEND | Pi `src/workers/*`, `lifecycle/roleRunner.ts`, `platform/WorkGraph.ts`, `InspectorFeed.parent_agent_id`. | No typed edges (spawn/handoff/delegation/consultation/…). Emit lineage events with stable ids. (T02) |
| 6 | Handoff / session rollover / checkpoint | REUSE + EXTEND | Pi `phaseBoundaryHandoff.ts`, `plannerWorker/sessionHandoff.ts`, `runtime/host/{handoffResume,missionHandover}.ts`, `orchestration/checkpoints.ts`. | Stable ids across host migration untested; events not in canonical envelope. (T02) |
| 7 | Mission submit/observe/cancel | REUSE | Pi `src/orchestration/externalEntry.ts` (+ externalAdmission/StartSelection/Supervision), tests external-entry-*. | Currently exits 2 unless `PI_AUTOSPEC_ENABLED`; needs a non-AutoSpec entry. (T03) |
| 8 | Risk/complexity planning | REUSE + EXTEND | Pi `src/lifecycle/classification.ts`, `policy.ts`, `orchestration/missionPlanner.ts`. | No explicit complexity score; plan revision not recorded as graph node. (T03) |
| 9 | Plannotator | EXTEND | Pi `src/platform/Plannotator.ts` (`PlannotatorAdapter`, PendingPlanStore, BypassAudit — referenced only inside src/platform, not wired into orchestrator). Herdr `plannotator.py` + `control_plane/reviews.py` (digest-bound receipts, 90s stale heartbeat). | Pi: no digest, no "offer" mode, no changes-requested replan loop, not wired. Herdr: no skip/offer/require policy. Policy lives in Pi; Herdr stores receipts. (T03) |
| 10 | Standalone-without-AutoSpec proof | NEW | Pi AGENTS.md/INV-013 require it. | No test proves it. (T03, acceptance 1) |
| 11 | AutoSpec protocol / Pi adapter | EXTEND | autospec `schemas/autospec-protocol-*.v1.schema.json`, `crates/autospec-core/src/protocol/*`, ADR 0004; Pi `src/adapters/AutoSpecAdapter.ts`. | No runtime Pi client in autospec. (T04) |
| 12 | Issue DAG | REUSE | autospec `initiative/dag.rs` (`TaskGraph`, `cross_repository_edges`), `dependency_gates.rs`, `coordination/ready_queue.rs`. | Confirm `owner/repo#N` linking; project into Herdr graph. (T05) |
| 13 | Constitution/Baselines/Design gates | REUSE | `autospec-design/rules.yaml` (24 rules, wcag), baselines `motion.reduced-motion`, `scripts/autospec-design-gates.sh` → `.autospec/reports/design-gates.json`. | Wire consumer in Pi/Herdr; visual rules are advisory. (T04, T16) |
| 14 | PR creation / issue linkage in Pi | NEW | none in Pi `src/merge/*` (check ingestion, merge gate, SHA fence only). | (T05) |
| 15 | Post-PR AI review | EXTEND | autospec `independent-review-adapter.sh` (repo/issue/PR/40-hex OID), `executor_bridge/review_receipt.rs` (rejects mismatched head), `autospec-guarded-merge.sh --verify-evidence`, `review_provider.rs` (`providers_are_diverse`). | Reuse these; do not write a new reviewer. (T06) |
| 16 | Quorum + stale-head invalidation | EXTEND + NEW | autospec `autospec-review-quorum.sh` → `autospec-autonomy-v3-lib.py review_quorum` (feature-scoped, no head SHA). Herdr `reviews.py` has no PR concept. | Add head_sha to quorum payload; add trigger that re-requests review when PR head changes; Herdr projects verdicts. (T06) |
| 17 | HCN rooms/membership/grants/audit/long-poll | REUSE | Herdr `control_plane/collab.py` (`CollabService`, `_effective`, `_authorize`, `room_state`), `collab_cognito.py`, ~20 collab tests. | None for baseline. |
| 18 | Room↔workspace link | NEW | Herdr only has a per-agent group label and `session-workspace-model.js linkedAgents`. | Link entity, one primary room per workspace, multi-room workspaces. (T07) |
| 19 | Tabs/panes lineage | EXTEND | Join script lays out `herd-PREFIX` workspace with a tab per agent. | Primary agents→tabs, subagents→panes, pane close ≠ kill. (T08) |
| 20 | A2A / MCP | EXTEND | `collab_a2a.py` (`message/send` text only, idempotent on messageId); `collab_mcp.py` (8 tools incl. task_create/task_update); REST task model exists. | Add accept/decline/progress/result/cancel + artifact proposals on the EXISTING task store; no budget/TTL/max-turns/hop fields yet. (T10, T11) |
| 21 | External agents / virtual tabs | NEW | HCN enrollment + join tokens exist. | Virtual tab with externally reported status, no terminal. (T10) |
| 22 | Graph UI | EXTEND | `packages/collab-flow.js` + `CollabService.room_flow` (SVG nodes + task/spawn edges), `herdr-timeline` component, `ci-pipeline.js`. | No Gantt, lineage graph, replay, inspectors. (T09) |
| 23 | Comments / steering / pause / cancel / retry | REUSE + NEW | Pi `task.steered` (missionStore.ts), `interrupt.ts`, `missionCancel.ts`, supervisorBackoff. | No authorized remote command path Herdr→Pi; no safe-boundary contract; no node-attached comments. (T12) |
| 24 | Fork / alternatives / promote | EXTEND + NEW | Pi `EngineeringRuntime.tournament()` (immutable incumbent INV-003); Herdr `candidates.py` (unwired, no compare). | Fork-from-node, approach id, impact check, stale-review invalidation, CAS promotion. (T13) |
| 25 | Contextual chat on nodes | EXTEND | HCN messages/threads. | Attach thread to graph node id. (T14) |
| 26 | Attention inbox | EXTEND | Herdr `attention.py` `AttentionCenter` (in-memory). | Durable store; graph-aware. (T15) |
| 27 | Analytics / ETA | NEW | none. | accepted work per token, handoff latency, … from real events only. (T15) |
| 28 | Security (intersection of grants, delegation bounds, redaction) | EXTEND | HCN `_effective`; room access ≠ terminal access exists conceptually. | Delegation TTL/budget/hops/cycle detection; traversal/replay/search authz. (T16) |
| 29 | CI | NEW (infra) | Pi: no CI config. Herdr: only PR/issue templates; CI external (Woodpecker, see docs/ci-and-reviews.md). | Follow each repo's actual gate; document in PRs. |
| 30 | Federation | NEW (ADR only) | n/a | T17 ADR, no implementation. |

## Consolidation decisions (no competing services)
- Single orchestrator: Pi. Single event store: Herdr `fabric_events`. Single collaboration service: HCN `CollabService`. Single dashboard: Herdr native web.
- Dead pieces to resolve rather than extend: `pi_autospec.py` (unwired), unused SQL tables (`reviews`, `review_findings`, `pull_requests`, `plans`, `plan_revisions`, `agents`, `agent_runs`) in migration 0001 — must be wired or documented before adding parallel ones (T01/T05).

## Per-repo contribution rules found
- Pi: branch+PR, one agent one worktree, `npm run typecheck`, biome lint, targeted `node --test --test-force-exit --import ./test/support/isolatedState.ts <file>`.
- Herdr: PR names phase/ownership/tests/exit-gate/compat/rollback; `STATUS.md` only after exit gate; `PYTHONPATH=. pytest tests/control_plane/...`; `npm run test:client`.
- AutoSpec: `feat/*` branch in linked worktree via scripts/worktree-guard.sh, conventional commits, `changelog.d/<issue>-<slug>.md` (do not edit CHANGELOG.md), new logic in Rust, PR template (Summary/Validation/Risk).
