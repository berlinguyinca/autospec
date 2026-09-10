# `endpoint-grant.v1` — endpoint grant + re-resolution contract (issue #3746)

Single source of truth for how dispatch hands an agent an inference endpoint.
Every dispatcher in this repo and the external worker tooling pin to the
shapes below.

- **Issue:** #3746 — agents are dispatched to a single worker endpoint by
  value at launch; when that worker is preempted (Slurm `low` partition,
  `PreemptMode=REQUEUE`), the agent dies with "Connection error" and the run
  is lost.
- **Issue:** #3758 — the stall watchdog kills runs that are still queued
  for a slot under fleet saturation: dispatch must not admit a run to a
  worker with zero free slots, and a watchdog kill must record the
  endpoint's queue state at the moment of firing so a stalled-while-queued
  run is distinguishable from a stalled-while-idle one.
- **Implementation:** [`crates/autospec-core/src/execution/endpoint.rs`](../../crates/autospec-core/src/execution/endpoint.rs)
  (pure module: no I/O, no wall-clock reads — callers supply `now`).
- **Scope of this doc:** the grant shape, the pool resolution and
  slot-admission rules, the agent-budget rule, the re-resolution protocol,
  the queue-state snapshot, and the `status.txt` reason tokens. Worker
  registration, Slurm accounting, and the gateway deployment itself are out
  of scope.

## Why

A dispatched agent holding a concrete worker reference by value is a value
that outlives the resource guarantee: the scheduler's guarantee (the
`low` partition's 130 s grace window) can end while the agent still holds
the endpoint. The contract fixes that two ways; a deployment uses whichever
it supports:

1. **Gateway fronting** (preferred): the pool is fronted by a stable
   gateway URL. Agents hold the gateway reference, never a worker
   reference, so a preemption behind the gateway is invisible to them.
2. **Bounded re-resolution**: agents hold a direct worker grant. On
   connection loss the dispatcher records the dead worker as unreachable
   and re-dispatches against *another healthy worker of the same model*,
   bounded by attempt count.
3. **Budget derived from the allocation**: the agent's wall-clock budget
   is derived from the grant's `valid_until` minus a flush margin, and an
   explicit budget that would outlive the allocation fails at submit time
   (issue #3613).
4. **Slot admission**: a worker serves a bounded number of concurrent runs
   (`total_slots`); a run is dispatched only while a slot is free, and a
   watchdog kill records the queue state so queued and idle stalls are
   distinguishable (issue #3758).

Re-resolution is **not** provider fallback. Spec §15's fail-closed rule
still binds at the executor: an executor never re-routes between providers
(a GPU failure must not silently become a cloud dispatch). Re-resolution
stays inside one model — same model, different worker — and a settled
failure from a live worker is never re-resolved.

## Grant shape

`Grant` is the only value dispatch may hand the agent at launch:

```json
{"kind": "gateway", "url": "http://gw:8080"}
```

```json
{"kind": "direct", "worker_id": "w-03", "endpoint": "http://gpu-node-3:8080", "valid_until": 1755350400}
```

| field | required | value |
|---|---|---|
| `kind` | yes | `gateway` \| `direct` |
| `url` | when `gateway` | stable gateway URL; no expiry horizon |
| `worker_id` | when `direct` | pool-unique worker id the grant was minted for |
| `endpoint` | when `direct` | concrete worker endpoint URL |
| `valid_until` | when `direct` | epoch second; **must equal the worker's `guaranteed_until` at mint time, never exceed it** |

**Grant-lifetime rule.** No dispatch may hand out a value that outlives the
resource's guarantee. `EndpointPool::resolve` refuses to mint a direct
grant for a worker whose `guaranteed_until <= now` (fails closed with
`NoCapacity`). Gateway grants carry no worker horizon: the gateway is
stable infrastructure, not a scheduler-managed resource.

## Agent budget and allocation (issue #3613)

The agent's wall-clock budget (`ExecutorRequest::timeout_secs`) and the
scheduler's allocation are **one decision, not two independent limits**
where the smaller one wins invisibly. `plan_agent_budget(grant, now,
explicit_secs, margin_secs) -> Result<u64, PoolError>`:

- `explicit_secs = None` **derives** the budget from the allocation:
  `valid_until - now - margin_secs`. There is no default budget constant —
  a budget that is not derived from the allocation does not exist.
- `explicit_secs = Some(b)` is used as-is when `b <= valid_until - now -
  margin_secs`; otherwise the dispatch fails at submit time with a
  `PoolError::Invalid` naming the explicit budget, the remaining
  allocation, and the margin. Eating the flush margin is a failure, not a
  rounding: the margin is the window in which partial output is flushed
  before the scheduler kills the job.
- `margin_secs` is the headroom between the agent stopping and the
  allocation ending; it is an explicit caller argument, never a hidden
  default.
- A **gateway grant** carries no allocation horizon, so neither derivation
  nor checking is possible: `plan_agent_budget` fails closed for it,
  explicit budget or not (spec §15).
- A direct grant whose `valid_until <= now` fails closed: there is no
  budget left to plan.

At dispatch start the dispatcher logs both limits via
`startup_limits_line(remaining_secs, agent_budget_secs)`, e.g.
`walltime=4h agent_limit=45m`.

## Pool resolution rules

Each worker registers with a `total_slots` capacity (≥ 1). A worker with
`total_slots = 0` fails closed at registration; pool state persisted
before the field existed deserializes to `0` and is rejected on
re-validation, so old records are never read as healthy capacity.

`EndpointPool::resolve(model, now)`:

- Fails closed with `NoCapacity { model }` when no registered worker is
  `healthy`, serves `model`, and has `guaranteed_until > now`.
- **Fails closed with `Saturated { model }` when healthy capacity exists
  but every slot is held** (issue #3758): a run is never dispatched to a
  worker with zero free slots. `Saturated` is distinct from `NoCapacity` —
  the fleet is healthy but full, and the run waits rather than dying.
- Hands out `Grant::Gateway` when a gateway is installed — always, even if
  workers are registered (the gateway front is authoritative).
- Otherwise hands out a `Grant::Direct` for the **deterministic** pick:
  the lexicographically smallest healthy worker id serving `model` **with
  at least one free slot**. Deterministic resolution keeps dispatch
  reproducible in tests and post-mortems.

Slot accounting (`in_use` per worker):

- `free_slots(worker_id)` — remaining slots on one worker; `None` for an
  unknown worker.
- `acquire(worker_id)` / `release(worker_id)` — the slot lifecycle a
  dispatch holds. `acquire` fails closed with `Saturated` when the worker
  has no free slot and with `UnknownWorker` for an unregistered id;
  `release` fails closed with `Invalid` when the worker holds no slot —
  driver-state corruption is not silently corrected.
- `free_slot_capacity(model, now)` — sum of free slots over healthy,
  guarantee-live workers serving `model`; the admission check `resolve`
  applies before it mints any grant.

`EndpointPool::re_resolve(model, failed_worker_id, now)`:

- Records `failed_worker_id` as `unreachable` first (which voids that
  worker's held slots), then resolves. The failed worker is never
  re-granted: it is unreachable by definition.
- Always resolves against the same `model` the run started with.

`mark_unreachable` is idempotent and the only way a worker leaves the
healthy set; it also clears the worker's slot accounting — the allocations
are voided with the worker, not leaked. The operator re-registers a worker
after it returns.

## Re-resolution protocol

`ReResolution` drives one run across attempts. The bound is on **attempt
count, not wall clock** (anti-loop guardrail, per AGENTS.md):

- `ReResolution::new(model, max_attempts)` — `max_attempts` is the total
  dispatch count including the first; default
  `ReResolution::DEFAULT_MAX_ATTEMPTS = 3` (first dispatch + two
  re-resolutions). Must be ≥ 1.
- `start(pool, now)` → first `Step::Dispatch { grant, attempt: 1 }` and
  **acquires a slot on the granted worker** (issue #3758): from this point
  until the run settles or the driver re-dispatches, the run counts in
  `in_use` — that is what makes the dispatch visible to admission.
- `on_outcome(pool, outcome, now)`:
  - `AttemptOutcome::Settled(status)` → releases the run's slot, then
    `Step::Halt { status }` immediately. A live worker's failure
    (provider error, timeout, timeout-partial, no-output) is never
    re-resolved.
  - `AttemptOutcome::ConnectionLost` on a **direct** grant → re-resolve
    against the same model excluding the dead worker and acquires the new
    worker's slot; dispatch again while `attempts < max_attempts` and
    free-slot capacity exists — a saturated fleet halts without spinning
    (`Saturated`); otherwise `Step::Halt { status: ConnectionError }`.
  - `AttemptOutcome::ConnectionLost` on a **gateway** grant →
    `Step::Halt { status: ConnectionError }` immediately. A dead gateway
    is an infrastructure failure, not worker preemption; re-dispatching
    the same gateway would spin.

When the budget or capacity runs out, the run halts and is recorded as
`connection-error` — the run stopped because its endpoint was lost, which
is the distinct fact the old "Connection error" death hid.

## `status.txt` reason tokens

`classify_status(&ExecutorResult)` maps a finished dispatch to a
`StatusReason`; the tokens below are what `status.txt` records. They are
**stable** — monitors and post-mortems branch on them.

| token | meaning | precedence |
|---|---|---|
| `connection-error` | `FailureClass::ProviderUnavailable`: the connection to the granted endpoint died (e.g. the worker was preempted), or re-resolution exhausted its budget/capacity | wins over empty output — a lost endpoint is not a no-output run |
| `timeout-partial` | `FailureClass::Timeout` and output or a patch was produced: the budget ran out with work to preserve (issue #3613) | a timeout is not a provider error and not `completed` |
| `no-output` | no failure class and neither output nor patch was produced, or `FailureClass::Timeout` without output or a patch | distinct from `connection-error` (issue #3746 acceptance) and from `timeout-partial` (issue #3613) |
| `no-output-queued` | watchdog termination with `held_slot = false`: the stall fired while the run was still waiting for a slot (issue #3758) | from `classify_watchdog_termination`, not `classify_status` |
| `no-output-idle` | watchdog termination with `held_slot = true`: the run held a slot and produced nothing (issue #3758) | from `classify_watchdog_termination`, not `classify_status` |
| `provider-error` | `FailureClass::ProviderError` \| `Unknown`: the dispatch failed at or beyond a live endpoint | — |
| `completed` | no failure class and output or a patch was produced | — |

`no-output-queued` and `no-output-idle` come from
`classify_watchdog_termination(&WatchdogTermination)`, not
`classify_status`: the watchdog fired before the executor finished, so
there is no `ExecutorResult` to classify. The discriminator is
`held_slot`, **not `free_slots`** — a run holding the last slot of a
saturated fleet sees `free_slots == 0` yet is idle; the slot the dispatch
side handed out is the admission fact (issue #3758).

`StatusReason::parse` fails closed on unknown tokens: a reader never
guesses a meaning for an unrecognized line.

## Queue state recording (issue #3758)

`EndpointPool::queue_state(model, now) -> QueueState` is a pure snapshot:

- `model` — the model the snapshot was taken for.
- `workers` — every healthy, guarantee-live worker serving `model`, in
  worker-id order, each as `WorkerOccupancy { worker_id, in_use,
  total_slots }`. **Fully occupied workers appear** — saturation is the
  interesting case, not the absence of workers.
- `free_slots` — sum of free slots over `workers`; `0` means the fleet is
  saturated and a new run can only wait.

A watchdog termination records the snapshot taken at the moment of firing
(`WatchdogTermination { held_slot, queue_state }`).
`WatchdogTermination::status_line()` renders it, e.g.
`status=no-output-queued held_slot=false free_slots=0 workers=w1:16/16,w2:0/4`
— workers as `worker_id:in_use/total` in snapshot order, `-` when the
snapshot is empty. That line is the post-mortem evidence that separates a
killed run that was queued from one that was idle on a slot.

## Failure taxonomy

| error | when |
|---|---|
| `PoolError::NoCapacity { model }` | no healthy worker with a live guarantee serves `model` at resolve time |
| `PoolError::Saturated { model }` | healthy workers with a live guarantee serve `model`, but every slot is held (issue #3758) — the run waits, it is not lost |
| `PoolError::UnknownWorker(id)` | `mark_unreachable` / re-resolution named a worker that was never registered |
| `PoolError::Invalid(msg)` | malformed entry or driver state: empty id/endpoint/model/url, duplicate worker id, `total_slots = 0`, a release of a slot the worker does not hold, `max_attempts < 1`; also budget planning failures: an explicit budget outrunning the allocation, a margin consuming the whole remainder, a dead allocation horizon, or a gateway grant (issue #3613) |

All failure paths are fail-closed: the pool never returns a stale or
cross-model grant, and the driver never dispatches without a grant.
