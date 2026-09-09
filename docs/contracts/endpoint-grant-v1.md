# `endpoint-grant.v1` — endpoint grant + re-resolution contract (issue #3746)

Single source of truth for how dispatch hands an agent an inference endpoint.
Every dispatcher in this repo and the external worker tooling pin to the
shapes below.

- **Issue:** #3746 — agents are dispatched to a single worker endpoint by
  value at launch; when that worker is preempted (Slurm `low` partition,
  `PreemptMode=REQUEUE`), the agent dies with "Connection error" and the run
  is lost.
- **Implementation:** [`crates/autospec-core/src/execution/endpoint.rs`](../../crates/autospec-core/src/execution/endpoint.rs)
  (pure module: no I/O, no wall-clock reads — callers supply `now`).
- **Scope of this doc:** the grant shape, the pool resolution rules, the
  re-resolution protocol, and the `status.txt` reason tokens. Worker
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

## Pool resolution rules

`EndpointPool::resolve(model, now)`:

- Fails closed with `NoCapacity { model }` when no registered worker is
  `healthy`, serves `model`, and has `guaranteed_until > now`.
- Hands out `Grant::Gateway` when a gateway is installed — always, even if
  workers are registered (the gateway front is authoritative).
- Otherwise hands out a `Grant::Direct` for the **deterministic** pick:
  the lexicographically smallest healthy worker id serving `model`.
  Deterministic resolution keeps dispatch reproducible in tests and
  post-mortems.

`EndpointPool::re_resolve(model, failed_worker_id, now)`:

- Records `failed_worker_id` as `unreachable` first, then resolves.
  The failed worker is never re-granted: it is unreachable by definition.
- Always resolves against the same `model` the run started with.

`mark_unreachable` is idempotent and the only way a worker leaves the
healthy set; the operator re-registers a worker after it returns.

## Re-resolution protocol

`ReResolution` drives one run across attempts. The bound is on **attempt
count, not wall clock** (anti-loop guardrail, per AGENTS.md):

- `ReResolution::new(model, max_attempts)` — `max_attempts` is the total
  dispatch count including the first; default
  `ReResolution::DEFAULT_MAX_ATTEMPTS = 3` (first dispatch + two
  re-resolutions). Must be ≥ 1.
- `start(pool, now)` → first `Step::Dispatch { grant, attempt: 1 }`.
- `on_outcome(pool, outcome, now)`:
  - `AttemptOutcome::Settled(status)` → `Step::Halt { status }`
    immediately. A live worker's failure (provider error, timeout,
    no-output) is never re-resolved.
  - `AttemptOutcome::ConnectionLost` on a **direct** grant → re-resolve
    against the same model excluding the dead worker; dispatch again while
    `attempts < max_attempts` and capacity exists; otherwise
    `Step::Halt { status: ConnectionError }`.
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
| `no-output` | no failure class, and neither output nor patch was produced | distinct from `connection-error` (issue #3746 acceptance) |
| `provider-error` | `FailureClass::ProviderError` \| `Timeout` \| `Unknown`: the dispatch failed at or beyond a live endpoint | — |
| `completed` | no failure class and output or a patch was produced | — |

`StatusReason::parse` fails closed on unknown tokens: a reader never
guesses a meaning for an unrecognized line.

## Failure taxonomy

| error | when |
|---|---|
| `PoolError::NoCapacity { model }` | no healthy worker with a live guarantee serves `model` at resolve time |
| `PoolError::UnknownWorker(id)` | `mark_unreachable` / re-resolution named a worker that was never registered |
| `PoolError::Invalid(msg)` | malformed entry or driver state: empty id/endpoint/model/url, duplicate worker id, `max_attempts < 1` |

All failure paths are fail-closed: the pool never returns a stale or
cross-model grant, and the driver never dispatches without a grant.
