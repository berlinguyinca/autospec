# ADR 0004 — Pi Engineering is the mission authority; the dispatcher becomes a library

- **Date:** 2026-10-14
- **Status:** Accepted
- **Satisfies:** `docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md` §4
  (authority decisions, items 1-7) and the §2a conflict resolutions.
- **Amends:** ADR 0002 (the sentence on where dispatch judgement lives).

## Context

Two source documents disagreed about who runs a mission. The master spec gave a
dual authority, with `autospec-orchestrator` and `autospec-dispatcher` running
managed missions. The second source made Pi Engineering the mission, merge-queue,
resource-lifecycle and recovery authority and did not mention the orchestrator or
the dispatcher at all (spec §2a, first row).

Left unresolved, that is the failure the counter-team named: two authorities able
to mutate one task, and a lease that expires being read as permission for the
second one to start. ADR 0002 also states that "dispatch judgement stays in
`autospec-dispatcher`", which would keep a second dispatcher alive on paper. This
ADR states each boundary once so that later issues can cite it.

## Decision

1. **Pi Engineering is the supervisor and the sole authority** for mission
   lifecycle, the serialized merge queue, resource ownership and cleanup, and
   recovery. AutoSpec owns the spec, task ids, dependencies and status
   integration.
   - Each task has exactly one *executor*: `pi-local` or `autospec-managed`.
   - Switching a task's executor requires quiescence of the old executor, durable
     reconciliation of what it left behind, and a fresh fencing epoch for the
     new one.
   - An expired lease alone never authorizes a second mutator. Expiry only makes
     a task eligible for the switch procedure above; until that procedure has
     completed, the old fencing epoch is the only valid one and writes carrying
     any other epoch are rejected.
2. **No new dispatcher or merge service.** The `autospec-dispatcher` repository
   becomes the `dispatcher-core` library (reviewer panel and independence
   policy), used by whoever reviews; its stub binary is removed. Dispatch
   scheduling stays in autospec (`initiative::dispatch`, `fleet_dispatch`).
   AutoSpec's own `/autospec-run` loop keeps merging AutoSpec issues; a Pi
   mission is merged only by Pi's queue.
3. **`autospec-orchestrator` is the optional managed executor**: worker,
   worktree and container lifecycle for the `autospec-managed` backend. It never
   owns a mission's merge, cleanup or recovery. `pi-engineering` is the
   interactive controller and supervisor. Product inference uses self-hosted
   open-weight models only, with no proprietary cloud fallback.
4. **The operator UI is HerdR Engineering.** `pi-engineering-web` stays a thin
   in-Pi panel over the same snapshot. `autospec-gui` gets no new features; it is
   fixed to the db schema or frozen. HerdR is a projection and control interface,
   not a second scheduler.
5. **`autospec-db` is telemetry only** and is never authoritative.
6. **The canonical model is the Initiative DAG** (`initiative/dag.rs`). The other
   DAG, lease and lifecycle types get adapters and a deprecation note, not a
   rewrite, in this effort.
7. **Required checks block; optional CI does not.** Local validation may precede
   CI but never replaces a required remote check or branch protection.

Resolved conflicts recorded with these decisions (spec §2a): the zip lifecycle
(`CREATED` through `ARCHIVED`, plus `BLOCKED`, `PAUSED`, `FAILED`, `CANCELLED`,
`RECOVERING`, `RETAINED_FOR_DIAGNOSTICS`) is canonical and is mapped additively
behind a schema version; reviewer independence is by model family; parallelism is
bounded by model slots, CPU/RAM/disk, build capacity and the merge queue.

## Consequences

- Later issues cite this ADR for "who may mutate a task". Any design in which two
  executors can write one task, or in which lease expiry grants write access, is
  a violation of decision 1.
- ADR 0002 is amended: dispatch scheduling stays in autospec, and the reviewer
  panel lives in `dispatcher-core`. The library move itself is tracked separately
  in the `autospec-dispatcher` repository (T12); this ADR changes no code.
- `autospec-orchestrator` keeps its worker mechanics but loses any claim to
  merge, cleanup or recovery of a Pi mission.
- The effort adds no dispatcher binary, merge service or second dashboard, and
  `autospec-gui` and `autospec-db` gain no authority.
