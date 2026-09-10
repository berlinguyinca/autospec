# Client timeout policy

Issue #3637: worker registration returned `000` from `curl` — no HTTP status at
all — for one worker, repeatedly, while that worker was healthy (`/health` →
200, Slurm job `RUNNING` for 10h) and three sibling workers registered `201` in
the same loop.

`000` reads as "the gateway is unreachable". The gateway was fine.

`POST /v1/workers` runs an **admission probe** server-side, and that probe
includes a one-token completion bounded by `defaultProbeTimeout = 30s`. The
sweep called it with `curl --max-time 20`. For a busy worker the admission
legitimately takes longer than 20s, so the client hung up mid-admission and
recorded a failure the server never saw. After raising the client bound:

```
-> 201 in 23.806979s
```

23.8s — comfortably over the old 20s limit and comfortably under the server's
30s. That worker had been failing to register on every sweep, silently, for
hours.

## The invariant

**A client timeout must exceed the server-side bound for the operation it
invokes.** Where it does not, a correct server produces a client-side error with
no server-side trace, and the resulting `000`/timeout is attributed to the wrong
component. A client bound below the server bound is a **configuration error, not
a tuning choice**.

`crates/autospec-core/src/service_timeout.rs` is the shared implementation:

| Item | Meaning |
|---|---|
| `ServerBound { operation, limit, class }` | The server's own limit for one operation, named as the server documents it. |
| `ADMISSION_PROBE` | `POST /v1/workers` admission probe, `limit = 30s`, class `GpuWork`. |
| `client_timeout_for(&bound)` | The client bound to use: `limit + margin`, where `margin` is half the server bound with a 5s floor. For `ADMISSION_PROBE` this is **45s**, the value that fixed #3637. |
| `validate_client_timeout(client, &bound)` | Rejects a configured client bound below `minimum_client_timeout()`, naming the operation, the server limit and the required minimum. Never silently clamp — a timeout that works "most of the time" is the failure mode above. |

```rust
use autospec_core::service_timeout::{client_timeout_for, validate_client_timeout, ADMISSION_PROBE};

let timeout = client_timeout_for(&ADMISSION_PROBE); // 45s for a 30s server bound
validate_client_timeout(timeout, &ADMISSION_PROBE)?;
```

Every client bound in a sweep, monitor or gateway caller must be **derived from**
one of these bounds (or documented against it in the caller), never picked by
intuition.

## GPU-work operations are called out

`OperationClass::GpuWork` marks operations whose duration depends on the device
queue rather than on the request: **admission probes, warmups, completions**.
Their server bounds are tens of seconds because a busy worker waits behind other
sessions for one token. `OperationClass::ControlPlane` marks request-handling
work (validation, metadata, writes).

Any new GPU-backed endpoint gets a `ServerBound` constant with
`class: OperationClass::GpuWork` and its server-side limit stated in the same
line, so the client bound is computed from a number.

## Transport failures vs HTTP errors

They have opposite causes and must never be logged identically:

- **Transport failure** (`000`, connection refused/reset, DNS, TLS, client
  timeout) — no response arrived. The server is not the suspect.
- **HTTP error status** (4xx/5xx) — the server saw the request and answered.
  This is the only variant that is evidence about the server.

`classify_call(http_status, curl_exit, client_bound)` splits them; exit code
wins over a missing status, because curl prints `000` precisely because no
status arrived. `CallFailure::server_saw_request()` and
`CallFailure::log_line(operation, target)` keep the attribution explicit:

```
operation=POST /v1/workers target=worker-7 transport=client_timeout http_status=none server_saw_request=false client_timeout=20s (check the server bound: ...)
operation=POST /v1/workers target=worker-7 transport=none http_status=503 server_saw_request=true
```

## Gateway documentation duty

A gateway must state the admission bound on the registration endpoint's own
documentation (upstream: `defaultProbeTimeout = 30s` on `POST /v1/workers`) so a
client configures against a number rather than a guess. Any change to a server
bound requires the matching `ServerBound` constant here to change in the same
PR, which moves every derived client bound with it.

Tests: `crates/autospec-core/tests/service_timeout.rs`.
