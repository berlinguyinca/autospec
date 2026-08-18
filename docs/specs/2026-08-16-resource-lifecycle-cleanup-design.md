# AutoSpec Resource Lifecycle, Cleanup, and Janitor Subsystem — design

**Status:** Implementation Specification — §47 Phase 1 decomposable; later phases sequenced below  
**Target Project:** AutoSpec  
**Primary Language:** Rust  
**Priority:** Critical / Core Infrastructure  
**Spec Version:** 1.0  
**Date:** 2026-08-16  
**Related:**
- `docs/specs/2026-08-16-autonomous-engineering-organization-design.md` (AS-AEO-001)
- `docs/specs/2026-08-16-repository-derived-real-work-benchmark-design.md`
- `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md`

## Measured justification

This specification describes a live, measured condition, not a hypothetical risk.
Taken from the primary working checkout on 2026-08-16:

| Resource | Observed |
|---|---|
| Local git branches | 6,475 |
| …already merged into `origin/main` | 5,926 |
| Git worktrees | 25 |
| Docker containers | 13, of which **0** carry an `autospec` label |
| `~/.autospec` state | 896 MB |

The zero-labeled-container figure is the important one: §15.1's labeling requirement
does not exist yet, so no current Docker resource can be attributed to a run. Every
container on the host is presently indistinguishable from a user-owned one, which is
exactly the condition Invariant 1 forbids acting on.

## Relationship to decisions already recorded

Checked against `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md`.

**§12's SQLite resource ledger is not a D3 violation.** D3 governs *routing and
dispatch telemetry* — an append-only stream of immutable events, where the JSONL
ledger is the system of record. The resource ledger is a different domain: mutable
lifecycle state with leases, heartbeats, cleanup-attempt counters and transactional
updates. That is not log-shaped and must not be forced into the routing ledger.

**But it must not become a second database.** D5 already accepted a database driver
and migration tooling in AS-AEO-001 Phase 1. The `resources` table therefore belongs
in **that** persistence layer, as additional tables and migrations — not a separate
SQLite file with its own driver, schema tooling and lifecycle. If this subsystem
lands before AS-AEO-001 Phase 1, its storage layer must be written so those tables
migrate into the shared database without a data migration.

**Ownership boundaries against AS-AEO-001:**

| Concern | Owner |
|---|---|
| Resource identity, leases, reconciliation, janitor, cleanup verification | This spec |
| Run/work-item lifecycle, policy, risk, approvals, emergency stop, budgets | AS-AEO-001 (§51, §69, Epic 12) |
| Resource hygiene as a benchmark dimension (§28, §29) | Scored here, ingested by the RealWork spec per D6 |

§29 benchmark integration is gated on the RealWork corpus existing.

## Existing surface this must reconcile with, not replace

Cleanup and lease behavior already exists in bash and must be inventoried before
Phase 2 reroutes creation:

- `~/.autospec/process-heartbeats/` — already path-scoped by repo slug to avoid
  cross-repository collision. The §19 lease system must preserve that scoping.
- `scripts/claim-guard.sh` and the claim layer in `crates/autospec-core/src/claim/`.
- `autospec-autonomous.sh cleanup` and the drain scripts.
- Per-session worktree isolation conventions already used by operators.

## Decomposition sequence

- **§47 Phase 1 (ledger + observation) is decomposable now.** It is read-only plus
  dry-run, introduces no deletion, and its first useful output is a report of the
  5,926 merged branches and 25 worktrees above. It depends on no open or parked work.
- **Phase 2 (managed creation)** follows Phase 1 and should land after the §36-style
  inventory above, so it reroutes existing creation paths rather than duplicating them.
- **Phases 3–5 (cleanup, crash recovery, janitor) must not be decomposed until the
  §42 git-safety and §45 property/invariant tests exist.** With 5,926 deletable
  branches on one host, a false positive in ownership detection destroys work.
  Invariants 1, 2, 3 and 5 are the acceptance bar, not aspirations.
- **Phase 6 (benchmark/UI)** waits on the RealWork corpus (D6).

---

## 1. Executive Summary

AutoSpec currently risks leaving behind resources created during planning, implementation, testing, review, benchmarking, and orchestration. Examples include:

- Git worktrees
- Local Git branches
- Remote tracking references
- Docker containers
- Docker images
- Docker volumes
- Docker networks
- Build cache
- Temporary directories and files
- Child processes
- Reserved ports
- Test databases and runtime artifacts

This specification introduces a mandatory **Resource Lifecycle Manager**, persistent **Resource Ledger**, crash-safe **Lease System**, periodic **Janitor**, startup **Reconciliation Engine**, and cleanup verification barrier.

The goal is simple:

> AutoSpec must finish engineering work in a known, clean, reproducible state and must safely reclaim AutoSpec-owned resources after success, failure, cancellation, or crash.

Cleanup becomes part of the definition of done rather than best-effort shell scripting.

---

# 2. Problem Statement

AutoSpec performs work through multiple agents, models, tools, worktrees, branches, containers, test environments, and temporary runtime resources.

Today, these resources may outlive the task that created them.

Common failure modes include:

1. Worktree created but never removed.
2. Local feature branch remains after PR merge.
3. Docker container exits but remains present.
4. Temporary Docker image remains indefinitely.
5. Docker volume survives because container teardown failed.
6. Docker Compose project is partially removed.
7. Build cache grows without bounds.
8. AutoSpec crashes before cleanup executes.
9. Cleanup code cannot determine whether a resource is still in use.
10. Cleanup scripts risk deleting resources not owned by AutoSpec.
11. Dirty worktrees or unpushed commits may be accidentally removed.
12. Multiple concurrent runs race to clean shared resources.
13. Ports remain reserved because child processes survive.
14. Temporary benchmark resources accumulate over repeated runs.

This creates:

- Disk exhaustion
- Confusing Git state
- Docker pollution
- Unpredictable test behavior
- Port conflicts
- Performance degradation
- Risk of deleting valuable work
- Reduced operator trust
- Difficult debugging and recovery

---

# 3. Objectives

The subsystem MUST:

1. Track every managed resource created by AutoSpec.
2. Associate each resource with an owning run and work item.
3. Clean resources after successful completion.
4. Clean resources after failed or cancelled execution.
5. Recover and clean resources after AutoSpec crashes.
6. Reconcile resource state at startup.
7. Periodically detect abandoned resources.
8. Never globally delete user-owned resources by default.
9. Protect dirty worktrees and unpushed commits.
10. Support dry-run inspection.
11. Provide clear CLI and UI observability.
12. Report resource leaks as first-class execution outcomes.
13. Integrate resource hygiene into AutoSpec benchmarks.
14. Support multiple repositories and concurrent AutoSpec runs.
15. Be implemented as reusable Rust infrastructure, not ad-hoc shell cleanup.

---

# 4. Non-Goals

This subsystem MUST NOT initially:

- Act as a general-purpose system cleaner.
- Delete arbitrary Docker resources not created by AutoSpec.
- Delete arbitrary Git branches outside AutoSpec ownership rules.
- Remove user development worktrees.
- Kill arbitrary system processes.
- Clean package-manager caches unrelated to AutoSpec.
- Automatically run dangerous global commands such as `docker system prune -a`.

Global cleanup may eventually exist as an explicit administrative feature, but it MUST NOT be the default behavior.

---

# 5. Core Architectural Principle

Every resource must have:

- Identity
- Resource type
- Owner
- Lifecycle state
- Creation timestamp
- Last heartbeat or lease renewal
- Cleanup policy
- Cleanup status
- Safety metadata
- External locator

The fundamental rule is:

> If AutoSpec creates a resource, AutoSpec must immediately register ownership before the resource can be used.

Creation and registration must be treated as one logical operation.

---

# 6. High-Level Architecture

```text
                    AutoSpec Orchestrator
                             |
       +---------------------+----------------------+
       |                     |                      |
    Planner              Implementer              Tester
       |                     |                      |
       +---------------------+----------------------+
                             |
                    Resource Manager
                             |
       +-----------+---------+---------+-----------+
       |           |         |         |           |
   Worktrees    Branches   Docker   Processes    Temp
       |           |         |         |           |
       +-----------+---------+---------+-----------+
                             |
                      Resource Ledger
                             |
                     Lease / Heartbeat
                             |
                +------------+------------+
                |                         |
       Reconciliation Engine           Janitor
                |                         |
                +------------+------------+
                             |
                    Cleanup Verifier
```

---

# 7. Major Components

The implementation MUST contain the following logical components.

## 7.1 ResourceManager

Central API through which AutoSpec creates and releases resources.

Responsibilities:

- Create resources.
- Register resources.
- Update lifecycle state.
- Renew leases.
- Release resources.
- Call type-specific cleanup handlers.
- Prevent unsafe cleanup.
- Expose owned resources to orchestrator and UI.

Agents SHOULD NOT directly create managed resources when a ResourceManager API exists.

---

## 7.2 ResourceLedger

Persistent record of resources owned by AutoSpec.

Recommended storage:

```text
.autospec/
  state/
    resources.db
```

SQLite is preferred for concurrency, crash recovery, indexing, and atomic updates.

A run-local JSON snapshot MAY additionally be emitted for debugging:

```text
.autospec/
  runs/
    <run-id>/
      resources.json
```

SQLite is the source of truth.

---

## 7.3 ResourceLeaseManager

Ensures stale resources can be distinguished from active resources.

Each active resource has:

- Lease expiration
- Last heartbeat
- Owning process/run
- Optional worker identity

Active workers renew leases periodically.

Expired leases do NOT automatically authorize deletion. They make a resource eligible for reconciliation.

---

## 7.4 ReconciliationEngine

Runs:

- At AutoSpec startup
- After unexpected termination recovery
- Before destructive cleanup
- On explicit `autospec reconcile`
- As part of scheduled janitor runs

It compares the resource ledger against real external state.

Examples:

- Does the Git worktree still exist?
- Is a worktree dirty?
- Does the Docker container still exist?
- Is the container running?
- Does the recorded process still exist?
- Is the PR merged?
- Is a branch commit safely reachable?

---

## 7.5 Janitor

Finds stale, orphaned, expired, completed, and reclaimable resources.

The Janitor MUST obey cleanup policy and ownership rules.

It SHOULD run:

- Periodically while AutoSpec daemon/orchestrator is active.
- During startup.
- After run completion.
- On explicit CLI request.

---

## 7.6 CleanupVerifier

A run MUST pass through cleanup verification before entering the final `COMPLETED` state.

Cleanup verification determines whether:

- All mandatory resources were released.
- Remaining resources are intentionally retained.
- Unsafe resources were quarantined.
- Leaks remain.

---

# 8. Run Lifecycle

Recommended run state machine:

```text
CREATED
  |
PREPARING
  |
RUNNING
  |
IMPLEMENTATION_COMPLETE
  |
TEST_COMPLETE
  |
REVIEW_COMPLETE
  |
ARTIFACTS_PERSISTED
  |
CLEANUP_PENDING
  |
CLEANING
  |
CLEANUP_VERIFICATION
  |
  +----------------------+
  |                      |
COMPLETED     COMPLETED_WITH_RESOURCE_LEAK
```

Failure path:

```text
RUNNING
  |
FAILED / CANCELLED
  |
CLEANUP_PENDING
  |
CLEANING
  |
CLEANUP_VERIFICATION
  |
FAILED_CLEAN | FAILED_WITH_RESOURCE_LEAK
```

Crash path:

```text
RUNNING
  |
PROCESS DISAPPEARS
  |
LEASE EXPIRES
  |
STARTUP / JANITOR RECONCILIATION
  |
ABANDONED
  |
CLEANING
```

---

# 9. Resource Types

Initial implementation MUST support:

```rust
pub enum ResourceType {
    GitWorktree,
    GitBranch,
    DockerContainer,
    DockerImage,
    DockerVolume,
    DockerNetwork,
    DockerComposeProject,
    ChildProcess,
    TempDirectory,
    TempFile,
    PortReservation,
    BuildCache,
}
```

Future resources SHOULD be extensible without redesigning the persistence model.

---

# 10. Resource Record

Recommended Rust model:

```rust
pub struct ManagedResource {
    pub id: ResourceId,
    pub run_id: RunId,
    pub work_item_id: Option<String>,
    pub repository_id: Option<String>,
    pub worker_id: Option<String>,

    pub resource_type: ResourceType,
    pub external_id: String,

    pub state: ResourceState,
    pub ownership: OwnershipClass,
    pub cleanup_policy: CleanupPolicy,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,

    pub cleanup_attempts: u32,
    pub last_cleanup_error: Option<String>,

    pub metadata: serde_json::Value,
}
```

Recommended states:

```rust
pub enum ResourceState {
    Creating,
    Active,
    Retained,
    CleanupPending,
    Cleaning,
    Released,
    Missing,
    Quarantined,
    CleanupFailed,
    Orphaned,
}
```

---

# 11. Ownership Classes

```rust
pub enum OwnershipClass {
    RunExclusive,
    RepoShared,
    GlobalShared,
    External,
}
```

Meaning:

### RunExclusive
Owned by exactly one AutoSpec run.

Examples:

- Worktree
- Task branch
- Temporary container
- Test volume

May generally be reclaimed after the run finishes.

### RepoShared
Shared by multiple AutoSpec runs in one repository.

Examples:

- Repository-level cache
- Shared base image

May only be reclaimed after reference count / lease checks.

### GlobalShared
Shared across multiple repositories.

Examples:

- Common model/runtime image
- Build cache

Requires explicit policy.

### External
Referenced but not created by AutoSpec.

MUST NOT be deleted.

---

# 12. Persistent Resource Ledger

SQLite schema SHOULD resemble:

```sql
CREATE TABLE resources (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    work_item_id TEXT,
    repository_id TEXT,
    worker_id TEXT,

    resource_type TEXT NOT NULL,
    external_id TEXT NOT NULL,

    state TEXT NOT NULL,
    ownership TEXT NOT NULL,
    cleanup_policy_json TEXT NOT NULL,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    lease_expires_at TEXT,
    last_heartbeat_at TEXT,

    cleanup_attempts INTEGER NOT NULL DEFAULT 0,
    last_cleanup_error TEXT,

    metadata_json TEXT NOT NULL
);

CREATE INDEX idx_resources_run_id
ON resources(run_id);

CREATE INDEX idx_resources_state
ON resources(state);

CREATE INDEX idx_resources_type
ON resources(resource_type);

CREATE INDEX idx_resources_lease
ON resources(lease_expires_at);
```

Writes MUST use transactions.

---

# 13. Git Worktree Management

## 13.1 Mandatory Rule

Agents MUST NOT create worktrees with arbitrary shell commands if the ResourceManager worktree API is available.

Preferred API:

```rust
let worktree = resource_manager
    .create_git_worktree(CreateWorktreeRequest {
        run_id,
        repository,
        branch,
        purpose,
    })
    .await?;
```

---

## 13.2 Worktree Naming

Recommended location:

```text
<repo>/.autospec/worktrees/<run-id>/<purpose>/
```

or centrally:

```text
~/.autospec/worktrees/<repo-id>/<run-id>/<purpose>/
```

Names MUST be deterministic and AutoSpec-identifiable.

Example:

```text
.autospec/worktrees/as-20260816-221904-a31f/implement/
```

---

## 13.3 Worktree Cleanup Safety Checks

Before deleting a worktree, AutoSpec MUST check:

1. Worktree still exists.
2. No active AutoSpec worker owns it.
3. No active lease remains.
4. Git status is clean OR explicit safe handling succeeds.
5. No valuable untracked files would be lost.
6. No unpushed commit would become unreachable.
7. Associated commit is recoverable.
8. Associated branch/PR state has been reconciled.
9. No known child process still uses the directory.

---

## 13.4 Dirty Worktrees

Dirty worktrees MUST NOT be silently deleted.

Default behavior:

```text
DIRTY -> QUARANTINE
```

Quarantine location:

```text
.autospec/quarantine/worktrees/<timestamp>-<run-id>/
```

Quarantine metadata MUST describe:

- Original path
- Git status
- Branch
- HEAD SHA
- Untracked files
- Owning run
- Reason for quarantine

The operator can later inspect and remove it.

---

## 13.5 Standard Worktree Cleanup

Normal cleanup SHOULD perform:

```bash
git worktree remove <path>
git worktree prune
```

The exact implementation should prefer safe Rust process invocation rather than shell interpolation.

---

# 14. Local Git Branch Management

## 14.1 Naming Convention

AutoSpec-owned branches SHOULD use a recognizable namespace:

```text
autospec/<work-item>/<role>/<slug>
```

Examples:

```text
autospec/417/implement/resource-manager
autospec/417/test/resource-manager
autospec/417/review/resource-manager
```

---

## 14.2 Branch Lifecycle

```text
CREATED
  |
ACTIVE
  |
PUSHED
  |
PR_OPEN
  |
MERGED
  |
RECLAIMABLE
  |
DELETED
```

Alternate terminal states:

```text
ABANDONED
QUARANTINED
RETAINED
```

---

## 14.3 Branch Cleanup Conditions

A merged branch may be automatically deleted locally only when:

1. It matches AutoSpec ownership rules.
2. No active worktree uses it.
3. PR merge state is known.
4. Its commits are reachable from a protected or remote branch.
5. It contains no unique unpushed commits.
6. Configured grace period has expired.

Default grace period:

```text
24 hours
```

Abandoned AutoSpec branches MAY be cleaned after a longer default period:

```text
7 days
```

---

## 14.4 Git Reconciliation

The Janitor SHOULD perform or request:

```bash
git fetch --prune
git remote prune origin
git worktree prune
```

before branch cleanup evaluation.

These operations MUST be repository-scoped.

---

# 15. Docker Resource Management

Docker cleanup MUST be based on ownership labels and/or deterministic project identifiers.

Global destructive Docker pruning is forbidden by default.

---

## 15.1 Required Labels

Every AutoSpec-created Docker object that supports labels MUST include:

```text
autospec.managed=true
autospec.run_id=<run-id>
autospec.repo=<repo-id>
autospec.work_item=<work-item>
autospec.purpose=<purpose>
autospec.created_at=<timestamp>
```

For shared resources:

```text
autospec.scope=shared
```

For temporary resources:

```text
autospec.scope=run
```

---

## 15.2 Docker Compose Projects

Every run-specific Compose stack MUST receive a unique project name.

Example:

```text
autospec_as_20260816_221904_a31f
```

Cleanup:

```bash
docker compose \
  -p autospec_as_20260816_221904_a31f \
  down --remove-orphans --volumes
```

AutoSpec MUST verify that the project belongs to the run before teardown.

---

## 15.3 Containers

Run-exclusive containers SHOULD be removed at run completion.

Cleanup policy:

```text
running container
  -> stop gracefully
  -> wait configured timeout
  -> force kill only if allowed
  -> remove
```

The cleanup subsystem MUST NOT kill containers lacking AutoSpec ownership evidence.

---

## 15.4 Images

Temporary run-specific images SHOULD use tags such as:

```text
autospec/tmp:<run-id>
autospec/test:<run-id>
```

Images MAY be removed when:

- No active container references them.
- They are AutoSpec-managed.
- Their retention policy has expired.
- They are not marked as shared cache/base images.

Reusable images SHOULD be marked:

```text
autospec.cache=shared
```

and retained according to cache policy.

---

## 15.5 Volumes

Volumes created for a run SHOULD be labeled and removed after successful teardown unless explicitly retained.

Database/test volumes MUST be treated as potentially valuable if a failed test asks for postmortem retention.

Policy should support:

```text
retain_on_failure = true
retain_for = "24h"
```

---

## 15.6 Networks

Run-exclusive networks SHOULD always be removed after all dependent containers are gone.

---

## 15.7 Build Cache

Build cache MUST be bounded.

Possible policy:

```toml
[cleanup.docker.build_cache]
enabled = true
max_age = "14d"
max_size = "50GB"
managed_only = true
```

Implementation MUST avoid deleting unrelated user build cache unless the user explicitly opts into broader cleanup.

---

# 16. Process Management

Any long-lived process spawned by AutoSpec MUST be registered.

Examples:

- Dev server
- Test server
- Browser automation server
- LLM runtime
- Local API
- Watch process
- Background compiler

Tracked metadata SHOULD include:

- PID
- Process start timestamp
- Parent PID
- Command hash
- Working directory
- Run ID
- Purpose
- Expected port(s)

PID alone MUST NOT be treated as sufficient identity because PIDs may be reused.

Cleanup SHOULD verify identity before termination.

---

# 17. Port Reservations

AutoSpec SHOULD track ports it reserves or binds for run-scoped services.

Port records should include:

```text
port
protocol
run_id
owning_process
purpose
```

At cleanup verification, AutoSpec SHOULD check that run-exclusive ports are no longer occupied by processes it owns.

---

# 18. Temporary Files and Directories

Run-scoped temporary data SHOULD be placed under deterministic directories:

```text
.autospec/tmp/<run-id>/
```

or:

```text
~/.autospec/tmp/<repo-id>/<run-id>/
```

This allows reliable cleanup without scanning arbitrary `/tmp` contents.

Temporary resources MUST still be registered if they contain important intermediate artifacts.

---

# 19. Lease and Heartbeat System

Each active run-exclusive resource SHOULD have a lease.

Example:

```json
{
  "owner": "as-20260816-221904-a31f",
  "lease_expires": "2026-08-17T05:00:00Z",
  "last_heartbeat": "2026-08-17T04:55:00Z"
}
```

Recommended defaults:

```text
heartbeat interval: 60 seconds
lease duration: 5 minutes
```

A resource becomes reconciliation-eligible when:

```text
lease expired
AND owning worker appears absent
```

It MUST NOT immediately be deleted solely because the lease expired.

---

# 20. Crash Recovery

On startup AutoSpec MUST:

1. Open the resource ledger.
2. Find resources in non-terminal states.
3. Reconcile their owners.
4. Determine whether the owning run is active.
5. Check leases.
6. Query actual Git/Docker/process state.
7. Classify resources:
   - active
   - orphaned
   - stale
   - missing
   - unsafe
   - reclaimable
8. Execute policy-based cleanup.
9. Record outcomes.

This startup reconciliation MUST occur before AutoSpec starts substantial new work, unless explicitly disabled.

---

# 21. Cleanup Policies

Example configuration:

```toml
[cleanup]
enabled = true
startup_reconciliation = true
shutdown_cleanup = true
periodic_janitor = true
janitor_interval = "30m"

[cleanup.worktrees]
remove_finished = true
stale_after = "24h"
dirty_action = "quarantine"

[cleanup.branches]
remove_merged = true
merged_grace_period = "24h"
delete_abandoned = true
abandoned_grace_period = "7d"

[cleanup.docker]
remove_stopped_containers = true
remove_run_volumes = true
remove_run_networks = true
remove_temporary_images = true
remove_shared_images = false

[cleanup.docker.build_cache]
enabled = true
max_age = "14d"
max_size = "50GB"

[cleanup.processes]
terminate_run_processes = true
graceful_timeout = "10s"
force_kill_after_timeout = true

[cleanup.temp]
remove_run_directories = true
stale_after = "24h"

[cleanup.safety]
managed_resources_only = true
preserve_dirty_worktrees = true
preserve_unpushed_commits = true
quarantine_unsafe_resources = true
```

---

# 22. Cleanup Ordering

Cleanup MUST respect dependencies.

Recommended order:

```text
1. Stop run-scoped child processes
2. Stop Docker services/containers
3. Remove Docker containers
4. Remove Docker networks
5. Remove Docker volumes
6. Remove temporary Docker images
7. Remove temporary files/directories
8. Remove Git worktrees
9. Reconcile Git references
10. Remove safe local branches
11. Run cleanup verification
```

Ordering may vary where dependency graphs require it.

The ResourceManager SHOULD eventually model dependencies explicitly.

---

# 23. Resource Dependency Graph

Recommended extension:

```rust
pub struct ResourceDependency {
    pub parent_resource_id: ResourceId,
    pub child_resource_id: ResourceId,
    pub relation: DependencyRelation,
}
```

Examples:

```text
compose-project -> container
container -> volume
container -> network
worktree -> branch
process -> port
```

Cleanup SHOULD occur in reverse dependency order.

---

# 24. CLI Requirements

## 24.1 `autospec cleanup`

Primary cleanup command.

```bash
autospec cleanup
autospec cleanup --dry-run
autospec cleanup --run <run-id>
autospec cleanup --issue <issue-id>
autospec cleanup --repo <repository>
autospec cleanup --stale
autospec cleanup --all-managed
```

Default behavior:

- Only managed resources.
- Respect safety checks.
- Quarantine unsafe resources.
- Produce structured summary.

---

## 24.2 `autospec reconcile`

```bash
autospec reconcile
autospec reconcile --dry-run
autospec reconcile --repo <repository>
autospec reconcile --run <run-id>
```

Purpose:

- Compare ledger state to actual resource state.
- Repair stale ledger records.
- Detect orphans.
- Identify reclaimable resources.

---

## 24.3 `autospec doctor resources`

Example output:

```text
AutoSpec Resource Health
────────────────────────────────────────

Worktrees
  active                  3
  stale                   7
  orphaned                2
  quarantined             1

Branches
  active                  6
  merged stale           19
  abandoned               4

Docker containers
  running                 2
  stopped                11
  orphaned                5

Docker images
  active                  4
  unused managed         13

Docker volumes
  active                  2
  stale                   6

Processes
  active                  5
  orphaned                1

Estimated reclaimable disk: 38.7 GB
```

---

## 24.4 `autospec resources`

Recommended inspection commands:

```bash
autospec resources list
autospec resources list --run <run-id>
autospec resources list --type worktree
autospec resources show <resource-id>
```

---

# 25. Dry-Run Behavior

Any destructive cleanup operation SHOULD support dry-run.

Dry-run MUST report:

- Resource
- Owner
- Current state
- Proposed action
- Safety reason
- Estimated reclaimed space when available

Example:

```text
WOULD REMOVE
  docker-container: autospec-test-a31f
  owner: as-20260816-221904-a31f
  reason: run completed 3h ago

WOULD QUARANTINE
  worktree: .../autospec/worktrees/.../implement
  reason: uncommitted changes detected
```

---

# 26. Quarantine

Unsafe-but-orphaned resources MUST support quarantine.

Initially required for Git worktrees.

Future support may include:

- Logs
- Test databases
- Artifact directories

Quarantine entries SHOULD be visible through:

```bash
autospec resources list --state quarantined
```

AutoSpec SHOULD support:

```bash
autospec cleanup quarantine --older-than 30d
```

with explicit safety confirmation or configured policy.

---

# 27. Definition of Done / Cleanup Barrier

AutoSpec MUST NOT mark a work item fully complete merely because implementation and tests succeeded.

Required lifecycle:

```text
IMPLEMENTATION_COMPLETE
        |
TEST_COMPLETE
        |
REVIEW_COMPLETE
        |
ARTIFACTS_PERSISTED
        |
RESOURCE_CLEANUP
        |
CLEANUP_VERIFIED
        |
TASK_COMPLETE
```

If cleanup does not fully succeed:

```text
COMPLETED_WITH_RESOURCE_LEAK
```

This state MUST be visible in:

- CLI
- Logs
- Run records
- Dashboard
- Benchmark output

---

# 28. Resource Hygiene Score

AutoSpec benchmark runs SHOULD measure cleanup quality.

Example:

```text
Resource Hygiene Score
────────────────────────────────
Orphan worktrees              0
Orphan branches               0
Running containers            0
Stopped containers            0
Temporary images              0
Orphan volumes                0
Orphan processes              0
Dirty resources destroyed     0

Score: 100 / 100
```

Possible scoring model:

```text
100 points starting score

-20 each destroyed dirty/valuable resource
-10 each orphan worktree
-5  each orphan process
-5  each orphan container
-3  each leaked volume/network
-2  each stale local branch
-1  each leaked temp directory
```

A destructive cleanup error SHOULD be more severe than a leak.

---

# 29. Benchmark Integration

Benchmark tasks SHOULD verify both engineering output and environmental hygiene.

A representative benchmark should:

1. Create task worktree.
2. Create task branch.
3. Build Docker image.
4. Launch test service.
5. Create volume/network.
6. Run tests.
7. Complete task.
8. Trigger cleanup.
9. Inspect environment.
10. Calculate Resource Hygiene Score.

Failure injection tests SHOULD include:

- Agent crash
- AutoSpec crash
- Docker command failure
- Dirty worktree
- Unpushed commit
- Stuck process
- Stale lease
- Partial Compose teardown
- Concurrent runs

---

# 30. Logging and Auditability

Every cleanup action MUST be auditable.

Recommended structured event:

```json
{
  "event": "resource.cleanup",
  "resource_id": "res_01J...",
  "resource_type": "DockerContainer",
  "run_id": "as-20260816-221904-a31f",
  "action": "remove",
  "result": "success",
  "duration_ms": 417,
  "timestamp": "2026-08-17T05:32:11Z"
}
```

Failures MUST include actionable diagnostics.

---

# 31. Metrics

Recommended metrics:

```text
autospec_resources_active
autospec_resources_orphaned
autospec_resources_quarantined
autospec_cleanup_attempts_total
autospec_cleanup_failures_total
autospec_cleanup_duration_seconds
autospec_reclaimed_bytes_total
autospec_worktrees_active
autospec_worktrees_stale
autospec_docker_containers_managed
autospec_docker_images_managed
autospec_branches_stale
autospec_processes_orphaned
```

Metrics SHOULD be available to AutoSpec observability/dashboard features.

---

# 32. UI / Dashboard Requirements

The AutoSpec UI SHOULD show a Resource Health panel.

Suggested summary:

```text
Resource Health: DEGRADED

Active        12
Stale          8
Orphaned       2
Quarantined    1
Reclaimable   34.2 GB
```

Drill-down SHOULD display:

- Resource type
- Owner run
- Issue/work item
- Repository
- Age
- State
- Proposed cleanup action
- Cleanup failure
- Quarantine reason

UI cleanup actions MUST invoke the same backend safety rules as CLI operations.

---

# 33. Concurrency Requirements

Multiple AutoSpec runs may execute concurrently.

Therefore:

- Ledger updates MUST be transactional.
- Resource ownership MUST be explicit.
- Cleanup MUST acquire appropriate locks.
- Shared resources MUST support reference tracking or leases.
- One run MUST NOT clean another active run's resources.
- Janitor operations MUST be idempotent.

SQLite WAL mode is recommended if SQLite is used.

---

# 34. Idempotency

Cleanup operations MUST be safe to retry.

Examples:

- Removing an already removed Docker container should resolve to `Released` or `Missing`.
- Removing an already pruned worktree reference should succeed logically.
- Repeated reconciliation must not create duplicate records.
- Cleanup failures must increment attempt count and preserve error history.

---

# 35. Failure Handling

Cleanup failures MUST NOT destroy run history.

Each failure should record:

- Resource
- Operation
- Exit code / API error
- Attempt count
- Timestamp
- Retry policy
- Safety classification

Transient cleanup errors SHOULD be retried with bounded exponential backoff.

Permanent/unsafe cleanup errors SHOULD transition to:

```text
Quarantined
```

or:

```text
CleanupFailed
```

---

# 36. Security and Safety Invariants

These are mandatory.

## Invariant 1

AutoSpec MUST NOT delete a resource it cannot reasonably establish ownership of.

## Invariant 2

Dirty Git worktrees MUST NOT be silently deleted.

## Invariant 3

Unique/unpushed Git commits MUST NOT be silently destroyed.

## Invariant 4

External resources MUST NOT be deleted.

## Invariant 5

Expired lease alone is insufficient proof for destructive cleanup.

## Invariant 6

Global Docker prune commands MUST NOT be the default cleanup mechanism.

## Invariant 7

Cleanup commands MUST avoid shell-string interpolation vulnerabilities.

## Invariant 8

Concurrent active runs MUST be protected from each other's cleanup.

---

# 37. Rust Module Structure

Recommended structure:

```text
src/
  resources/
    mod.rs
    manager.rs
    model.rs
    ledger.rs
    lease.rs
    policy.rs
    reconcile.rs
    janitor.rs
    verify.rs
    dependency.rs

    git/
      mod.rs
      worktree.rs
      branch.rs
      safety.rs

    docker/
      mod.rs
      container.rs
      image.rs
      volume.rs
      network.rs
      compose.rs
      cache.rs

    process/
      mod.rs
      child.rs
      identity.rs

    temp/
      mod.rs
      directory.rs
      file.rs

    port/
      mod.rs
      reservation.rs

  cli/
    cleanup.rs
    reconcile.rs
    resources.rs
    doctor_resources.rs
```

---

# 38. Trait Design

Suggested resource cleanup abstraction:

```rust
#[async_trait::async_trait]
pub trait ResourceHandler: Send + Sync {
    fn resource_type(&self) -> ResourceType;

    async fn inspect(
        &self,
        resource: &ManagedResource,
    ) -> Result<ResourceInspection>;

    async fn cleanup(
        &self,
        resource: &ManagedResource,
        policy: &CleanupPolicy,
    ) -> Result<CleanupOutcome>;
}
```

Inspection result:

```rust
pub struct ResourceInspection {
    pub exists: bool,
    pub active: bool,
    pub safe_to_remove: bool,
    pub reclaimable_bytes: Option<u64>,
    pub safety_reasons: Vec<String>,
    pub metadata: serde_json::Value,
}
```

---

# 39. Resource Creation Transaction Pattern

Preferred logical flow:

```text
1. Generate resource ID.
2. Insert ledger row in Creating state.
3. Create external resource.
4. Update external locator.
5. Mark Active.
6. Start lease heartbeat.
```

If step 3 fails:

```text
Creating -> CleanupFailed or Released
```

If external creation succeeds but ledger update fails, AutoSpec MUST immediately attempt compensating cleanup and emit a critical error.

---

# 40. Suggested Cleanup API

Example:

```rust
pub async fn release_resource(
    &self,
    resource_id: ResourceId,
) -> Result<CleanupOutcome>;
```

Bulk cleanup:

```rust
pub async fn cleanup_run(
    &self,
    run_id: RunId,
    options: CleanupOptions,
) -> Result<CleanupReport>;
```

---

# 41. Cleanup Report

Recommended:

```rust
pub struct CleanupReport {
    pub run_id: Option<RunId>,
    pub inspected: usize,
    pub removed: usize,
    pub retained: usize,
    pub quarantined: usize,
    pub failed: usize,
    pub missing: usize,
    pub reclaimed_bytes: u64,
    pub results: Vec<ResourceCleanupResult>,
}
```

---

# 42. Git Safety Tests

Required test cases:

1. Clean worktree removed successfully.
2. Dirty tracked file causes quarantine.
3. Untracked file causes quarantine.
4. Unpushed commit prevents destructive cleanup.
5. Merged branch deleted after grace period.
6. Active branch retained.
7. Branch used by active worktree retained.
8. Non-AutoSpec branch never deleted.
9. Missing worktree marked missing/released.
10. Concurrent cleanup is idempotent.

---

# 43. Docker Tests

Required cases:

1. Run container removed.
2. Stopped managed container removed.
3. Unmanaged container untouched.
4. Run volume removed.
5. Shared volume retained.
6. Run network removed.
7. Temporary image removed when unused.
8. Shared image retained.
9. Compose project cleaned correctly.
10. Partial teardown recovered by reconciliation.
11. Active run resources retained.
12. Concurrent runs cannot delete each other's resources.

---

# 44. Crash-Recovery Tests

Required scenarios:

1. Kill AutoSpec after worktree creation.
2. Kill AutoSpec after container startup.
3. Kill AutoSpec after branch push.
4. Restart AutoSpec.
5. Startup reconciliation identifies abandoned run.
6. Safe resources are reclaimed.
7. Dirty resources are quarantined.
8. Ledger ends consistent with actual environment.

---

# 45. Property / Invariant Tests

Where practical, add property tests proving:

- Cleanup is idempotent.
- Unmanaged resources are never selected for deletion.
- Active leased resources are not deleted.
- Cleanup ordering respects dependencies.
- Dirty worktrees never transition directly to deleted.
- `External` resources never receive destructive action.

---

# 46. Integration Test Environment

Create an isolated integration-test harness that can:

- Create temporary Git repos.
- Create worktrees.
- Create branches.
- Launch test containers.
- Create Docker networks/volumes.
- Spawn child processes.
- Simulate stale leases.
- Simulate crashes.

Tests MUST leave their own environment clean.

---

# 47. Migration / Rollout Strategy

## Phase 1 — Ledger and Observation

Implement:

- Resource model
- SQLite ledger
- Resource inspection
- `autospec resources`
- `autospec doctor resources`
- Dry-run cleanup

Do not enable broad automatic deletion yet.

## Phase 2 — Managed Creation

Route new resource creation through ResourceManager:

- Git worktrees
- Git branches
- Docker Compose projects
- Containers
- Volumes
- Networks
- Child processes

## Phase 3 — Normal Cleanup

Enable cleanup after successful and failed runs.

## Phase 4 — Crash Recovery

Enable:

- leases
- heartbeat
- startup reconciliation
- abandoned-run cleanup

## Phase 5 — Janitor

Enable periodic stale-resource cleanup.

## Phase 6 — Benchmark / UI

Add:

- Resource Hygiene Score
- resource dashboard
- leak reporting
- benchmark failure injection

---

# 48. Backward Compatibility

Existing resources created before this subsystem may not have ownership metadata.

AutoSpec MUST treat them conservatively.

Possible legacy detection MAY use:

- Branch prefix
- Known worktree path
- Docker names/tags
- Compose project prefix

But legacy resources MUST initially be reported rather than automatically deleted unless ownership confidence is high.

---

# 49. Configuration Defaults

Recommended defaults:

```toml
[cleanup]
enabled = true
startup_reconciliation = true
shutdown_cleanup = true
periodic_janitor = true
janitor_interval = "30m"

[cleanup.worktrees]
remove_finished = true
stale_after = "24h"
dirty_action = "quarantine"

[cleanup.branches]
remove_merged = true
merged_grace_period = "24h"
delete_abandoned = true
abandoned_grace_period = "7d"

[cleanup.docker]
remove_stopped_containers = true
remove_run_volumes = true
remove_run_networks = true
remove_temporary_images = true
remove_shared_images = false

[cleanup.processes]
terminate_run_processes = true
graceful_timeout = "10s"
force_kill_after_timeout = true

[cleanup.safety]
managed_resources_only = true
preserve_dirty_worktrees = true
preserve_unpushed_commits = true
quarantine_unsafe_resources = true
```

---

# 50. Acceptance Criteria

The feature is complete only when all of the following are true.

## Resource Tracking

- [ ] AutoSpec persistently tracks all supported resource types.
- [ ] Each resource has a run owner.
- [ ] Resource creation is registered immediately.
- [ ] Resource lifecycle transitions are persisted.

## Worktrees

- [ ] AutoSpec-created worktrees are registered.
- [ ] Clean completed worktrees are removed automatically.
- [ ] Dirty worktrees are quarantined.
- [ ] Unpushed commits cannot be silently destroyed.
- [ ] Stale worktree metadata is pruned.

## Branches

- [ ] AutoSpec branches use a recognizable namespace.
- [ ] Merged local branches are reclaimed after grace period.
- [ ] Branches used by active worktrees are retained.
- [ ] Non-AutoSpec branches are never automatically removed.
- [ ] Unpushed commits are protected.

## Docker

- [ ] AutoSpec Docker resources receive ownership labels.
- [ ] Compose projects use unique run identifiers.
- [ ] Run containers are removed.
- [ ] Run volumes are removed according to policy.
- [ ] Run networks are removed.
- [ ] Temporary images are reclaimed.
- [ ] Shared images remain protected.
- [ ] AutoSpec does not globally prune Docker by default.

## Processes

- [ ] AutoSpec child processes are tracked.
- [ ] Run-scoped processes are terminated after completion.
- [ ] PID reuse is handled safely.
- [ ] Orphaned processes are detected.

## Recovery

- [ ] Active resources have leases.
- [ ] Startup reconciliation runs automatically.
- [ ] AutoSpec can recover from a crash.
- [ ] Abandoned run resources are safely reclaimed.
- [ ] Unsafe resources are quarantined rather than destroyed.

## CLI

- [ ] `autospec cleanup` implemented.
- [ ] `autospec cleanup --dry-run` implemented.
- [ ] `autospec reconcile` implemented.
- [ ] `autospec doctor resources` implemented.
- [ ] Resource-specific filters implemented.

## Completion Semantics

- [ ] Cleanup is part of the run completion barrier.
- [ ] Resource leaks produce `COMPLETED_WITH_RESOURCE_LEAK`.
- [ ] Cleanup results are visible in logs and run records.

## Benchmarking

- [ ] Resource Hygiene Score implemented.
- [ ] Benchmark detects orphan worktrees.
- [ ] Benchmark detects Docker leaks.
- [ ] Benchmark detects orphan processes.
- [ ] Crash/failure cleanup benchmark exists.

---

# 51. Mandatory Safety Acceptance Tests

The following tests MUST pass before automatic cleanup is enabled by default:

```text
TEST: unmanaged Docker container survives cleanup
EXPECTED: PASS

TEST: user branch survives cleanup
EXPECTED: PASS

TEST: dirty AutoSpec worktree is quarantined
EXPECTED: PASS

TEST: unpushed AutoSpec branch survives destructive cleanup
EXPECTED: PASS

TEST: crashed AutoSpec run is reclaimed after reconciliation
EXPECTED: PASS

TEST: active concurrent run survives janitor
EXPECTED: PASS

TEST: cleanup can be run twice with identical safe outcome
EXPECTED: PASS
```

---

# 52. Operational Success Criteria

After implementation, a healthy completed AutoSpec run SHOULD normally result in:

```text
Orphan worktrees:          0
Stale AutoSpec branches:   0
Run containers:            0
Run networks:              0
Run volumes:               0
Run temporary images:      0
Run processes:             0
Run temp directories:      0
Dirty resources destroyed: 0
```

Shared caches may remain by explicit policy.

---

# 53. Future Enhancements

Possible follow-on capabilities:

- Disk usage budgets per repository.
- Per-agent resource quotas.
- Automatic cache LRU.
- UI button for safe cleanup preview.
- Resource dependency visualization.
- Disk pressure-triggered janitor runs.
- Prometheus/OpenTelemetry integration.
- Remote worker cleanup.
- Kubernetes pod/job cleanup.
- Cloud sandbox cleanup.
- Per-benchmark resource budget.
- Leak attribution by agent/model.
- Cleanup reliability included in model routing scores.

---

# 54. Implementation Guidance for AutoSpec

This work should be implemented as foundational infrastructure before adding more systems that create large numbers of worktrees, branches, containers, test environments, or benchmark resources.

Recommended implementation order:

```text
1. Resource models and SQLite ledger
2. Inspection-only reconciliation
3. ResourceManager worktree integration
4. Docker ownership labeling
5. Explicit cleanup CLI + dry-run
6. Worktree safe cleanup/quarantine
7. Docker cleanup
8. Branch cleanup
9. Process tracking
10. Lease/heartbeat
11. Startup recovery
12. Periodic janitor
13. Cleanup completion barrier
14. UI observability
15. Resource Hygiene benchmark
```

Each stage should be independently tested and should preserve the safety invariants in this specification.

---

# 55. Final Requirement

AutoSpec must adopt the following invariant as part of its orchestration model:

> A task is not fully complete until its intended artifacts are persisted, its temporary resources are cleaned or intentionally retained, cleanup has been verified, and no user-owned work has been destroyed.

This subsystem is mandatory infrastructure for reliable autonomous operation.
