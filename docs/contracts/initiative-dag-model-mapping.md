# Initiative DAG model mapping

Canonical model: the Initiative DAG, `initiative::dag::TaskGraph` with `initiative::task::{Task, TaskState, Lease}`
(ADR 0004, `docs/decisions/0004-pi-supervisor-authority-and-dispatcher-placement.md`; spec
`docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md` section 3 and section 4 item 6).

Every other DAG, lease and task-lifecycle type is mapped to the canonical one by an adapter, not rewritten. This
document is the contract the adapters must satisfy; the adapter modules named below are added by follow-up issues
(`initiative::compat_dag` #4759, `initiative::compat_lease` #4760, `initiative::compat_lifecycle` #4761) and do not
exist until those merge. `Task.affects` and `AffectedSurface` come from `initiative::collision` (#4756). Two rules hold
in every mapping:

- Fencing data is never dropped. A source value that carries a `fencing_generation` keeps it visible through the adapter.
- Nothing unknown becomes done. A state or edge the canonical model cannot place maps to a state that does not satisfy
  dependents (`TaskState::satisfies_dependents` is true only for `Verified`).

## DAG models

Source models: `graph::issue_dag::IssueGraph` (`waves`, `detect_cycle`), `graph::order::execution_order` over
`spec::SpecMetadata`. Canonical: `initiative::dag::TaskGraph` (`topological_order`, `concurrency_waves`, `schedule`).
Adapter (planned, #4759): `initiative::compat_dag`.

| Source field | Canonical field | Rule |
|---|---|---|
| `PlannedIssue.id` | `Task.id` | Ids are assigned `TASK-0001` upward in sorted source-id order, so the result is deterministic. |
| `PlannedIssue.title` | `Task.summary` | Copied. |
| `PlannedIssue.ownership.exclusive` | `Task.affects` | One `AffectedSurface` per entry; `path` and `symbols` kept; a glob `dir/**` becomes `dir/`. |
| `PlannedIssue.ownership.shared_write` | `Task.affects` | Same rule as `exclusive`. Shared writes are writes. |
| `PlannedIssue.ownership.shared_read` | not mapped | Reads do not collide; the canonical model has no read set. |
| `PlannedIssue.concurrency` | not mapped | Conflict risk is not a dependency; collision is predicted from `affects` instead. |
| `IssueGraph.hard_edges` (`predecessor`, `successor`) | `Task.depends_on` of the successor | An edge naming an id not in `issues` is a `GraphViolation::InvalidTask` naming the edge. The old model ignored such edges; the adapter does not. |
| `DependencyEdge.reason`, `DependencyEdge.artifact` | not mapped | Justification metadata with no scheduling effect. |
| `SpecMetadata.id` | `Task.id` | Same sorted-order assignment as `PlannedIssue.id`. |
| `SpecMetadata.title` | `Task.summary` | Copied. |
| `SpecMetadata.dependencies` | `Task.depends_on` | A dependency that is not a listed spec is a `GraphViolation::InvalidTask` naming it (`execution_order` returns `MissingDependency`; a canonical `MissingDependency` needs a `TaskId` an unknown id does not have). |
| `SpecMetadata.files_to_touch` | `Task.affects` | One `AffectedSurface` per entry, no symbols. |
| `GraphError { kind: Cycle }` and `IssueGraph::detect_cycle` | `GraphViolation::Cycle` | The adapter validates the built graph with `TaskGraph::topological_order`, so a cycle the old model rejected is rejected. |
| `IssueGraph::waves` | `TaskGraph::concurrency_waves` | Required: the same waves for the same graph, checked on a diamond. |
| `execution_order` | `TaskGraph::topological_order` | Required: the old order is a valid topological order of the adapted graph. |

All adapted tasks are `TaskKind::Implementation`, state `Defined`, in the repository and plan the caller names.

## Lease types

Source models: `resilience::work_protocol::Lease`, `claim::ClaimLease`. Canonical: `initiative::task::Lease`.
Adapter (planned, #4760): `initiative::compat_lease` (`LeaseView`, `may_supersede`).

| Source field | `LeaseView` accessor | Rule |
|---|---|---|
| `work_protocol::Lease.session_id` | `holder` | The session holding the lease. |
| `work_protocol::Lease.expires_at` | `expires_at` | Unix seconds. |
| `work_protocol::Lease.fencing_generation` | `fencing_generation` | `Some(n)`. Never dropped. |
| `work_protocol::Lease::is_expired` | `is_live` | `is_live(now)` is `!is_expired(now)`. |
| `ClaimLease.worker_id` | `holder` | The worker holding the claim. |
| `ClaimLease.updated_at + ClaimLease.ttl_seconds` | `expires_at` | Derived. |
| `ClaimLease` (no generation) | `fencing_generation` | `None`. The adapter never invents a generation. |
| `ClaimLease::is_stale_at` | `is_live` | `is_live(now)` is `!is_stale_at(now)`. A merged claim is terminal and stays live: it never permits a second mutator. |
| `task::Lease.session_name` | `holder` | The Pi session holding the lease. |
| `task::Lease.expires_at` | `expires_at` | Unix seconds. |
| `task::Lease` (no generation) | `fencing_generation` | `None`. Executor switching needs a fresh fencing epoch (spec section 4 item 1); until the canonical lease carries one, `may_supersede` refuses. |
| `task::Lease::is_live` | `is_live` | Unchanged. |
| `may_supersede(current, next_generation, now)` | n/a | `false` while `current` is live. After expiry, `true` only when `next_generation` is strictly greater than the current generation. A lease without a generation returns `false`: an expired lease alone never authorizes a second mutator. |

## Task lifecycles

Source models: `resilience::work_protocol::WorkState` (9) with `WorkStateNonHappy` (8). Canonical:
`initiative::task::TaskState` (12). Adapter (planned, #4761): `initiative::compat_lifecycle`. `non_happy` outranks `state`: when it is
set, the result comes from the non-happy table.

| Source state | `TaskState` | Rule |
|---|---|---|
| `WorkState::Created` | `Defined` | Known, not released. |
| `WorkState::Assigned` | `Ready` | Releasable. |
| `WorkState::Delivered` | `Leased` | Delivered to a worker, not started. Shares `Leased` with `Claimed`. |
| `WorkState::Claimed` | `Leased` | Claimed by a scheduler, not started. |
| `WorkState::Running` | `Running` | |
| `WorkState::Completed` | `AwaitingTest` | Implementation done; tests have not reported. |
| `WorkState::Validated` | `AwaitingReview` | Deterministic validation done; review has not reported. |
| `WorkState::Reviewed` | `AwaitingReview` | Reviewed but not merged. Does not satisfy dependents. Shares `AwaitingReview` with `Validated`. |
| `WorkState::Merged` | `Verified` | The only happy state that satisfies dependents. |
| `WorkStateNonHappy::Blocked` | `Blocked` | |
| `WorkStateNonHappy::Failed` | `FailedTerminal` | |
| `WorkStateNonHappy::Abandoned` | `FailedRetryable` | The worker is gone; another attempt may succeed. |
| `WorkStateNonHappy::RetryPending` | `FailedRetryable` | |
| `WorkStateNonHappy::NeedsHuman` | `Blocked` | Recoverable once a human acts; `FailedTerminal` has no way out. |
| `WorkStateNonHappy::Superseded` | `Superseded` | |
| `WorkStateNonHappy::Cancelled` | `FailedTerminal` | Not `Superseded`: nothing replaced it. |
| `WorkStateNonHappy::Unknown` | `Blocked` | Unknown never unblocks a dependent. |

Reverse direction (`work_state_from_task`): `Defined` to `Created`, `Ready` to `Assigned`, `Leased` to `Delivered`,
`Running` to `Running`, `AwaitingTest` to `Completed`, `AwaitingReview` to `Validated`, `Verified` to `Merged`, each with no
`non_happy`. `Blocked`, `FailedRetryable`, `FailedTerminal` and `Superseded` map to `Created` carrying `Blocked`,
`RetryPending`, `Failed` and `Superseded`: the `TaskState` does not record how far the work had got, so the
least-advanced `WorkState` is used. `ChangesRequested` has no counterpart (a review round is a new attempt in the work
protocol) and maps to nothing.

The reverse is never further along than the source, and the forward map is stable under it. Two happy states are
lossy because their `TaskState` is shared: `Claimed` returns as `Delivered`, and `Reviewed` returns as `Validated`.
Seven of the nine happy states return exactly as they went in.

## Not adapted

| Type | Why it is left alone |
|---|---|
| `worktree_lock::Lease` | A checkout mutex (worktree, holder, pid, phase) with no expiry or generation; it guards a directory, not a task. |
| `agent::session::Lease` | A session epoch for native agent sessions (holder, epoch); the epoch is not a task-scoped fencing generation and has no expiry. |
| `PortfolioLease` | A portfolio-level lease stored in a git ref by `autospec-cli`. It does carry a `generation` and `expires_at`, but they fence a portfolio plan, not a task: the generation never reaches a task lease and the fields are private to that module. |
| `LeaseGeneration` | A bare positive counter in autonomous accountability; it is a value, not a lease. |

## Deprecation policy

The non-canonical types stay. Their module and type documentation gains a note naming the canonical type and the
`initiative::compat_*` adapter. No `#[deprecated]` attribute is added, because it would turn every existing caller into
a warning, and no caller is rewritten in this effort.
