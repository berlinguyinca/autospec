# Reliability telemetry rollout: workers that answer `/health` but cannot generate

On 2026-09-11 all five `qwen3.8-flash-next` workers returned **HTTP 200 from
`/health`** while returning **HTTP 000 from an actual generation request**.
Nothing in the fleet treated that as a fault. The workers stayed registered and
routable, the gateway kept routing to them, and clients saw hangs rather than
errors — the worst measured request took 2976 seconds before anyone looked.

The failure is not that a worker broke. Workers break. The failure is that every
liveness signal the fleet had was still green while the thing a client actually
wants — a token — was unavailable. This runbook records how that class of fault
is detected now, what telemetry exists as a result, how to render it, and what
an operator does when it fires.

## Why `/health` is not a liveness check

`/health` is served by the worker's HTTP layer. The HTTP layer accepts, routes,
and answers while the generation path behind it is wedged, so a 200 proves the
process is scheduled and its socket is open and nothing more. A liveness check
that a client would recognise has to exercise the same path the client uses:
submit a completion and require a token back.

This is not a subtlety that was missed — it was encoded as a deliberate policy
in the wrong place. The hive gateway had already noticed the fault and decided
to ignore it: **237 `"probe timed out; leaving worker in the pool (busy is not
dead)"` warnings, every one of them `flash-next`**. The gateway's
`probeTimeout = 5 * time.Second` (`cmd/gateway/reconcile.go:15`) is short enough
that a genuinely loaded worker trips it routinely, so treating a timeout as
death would evict healthy workers under load. That reasoning is correct. It is
correct *for a 5-second probe*, and it was applied to a worker whose GPU was at
0% and which would not have answered in five minutes either.

The watchdog resolves this by changing the timeout rather than the verdict: it
probes with `GEN_TIMEOUT=45` seconds, nine times the gateway's window, and then
still requires three consecutive failures before acting. A slow worker is not a
dead one; a worker that cannot produce one token in forty-five seconds, three
sweeps running, is not slow.

## What the telemetry showed

Latency from `/var/lib/inferweave/telemetry.ndjson` on host `fry`, 762 rows
spanning 2026-09-10 22:43 to 2026-09-11 10:52:

| model | n | p50 | p90 | p99 | max | >60s | >120s | >300s |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| qwen3.8-flash-next | 217 | 377ms | 85.1s | 1239s | 2976s | 26 | 21 | 17 |
| qwen3.8-27b | 249 | 3366ms | 19.1s | 88.3s | 92.9s | 6 | 0 | 0 |
| qwen3.8-27b-vision | 139 | 593ms | 8.4s | 25.4s | 26.3s | 0 | 0 | 0 |
| deepseek-v4-flash | 138 | 83ms | 88ms | 104ms | 161ms | 0 | 0 | 0 |

100% of requests over 120 seconds were `flash-next`. The shape matters more than
the totals: `flash-next` has the **fastest p50 of the three qwen models** and by
far the worst tail. That is the signature of this fault and not of an overloaded
or undersized model. Requests that happened to land on the one healthy worker
returned in 377ms; requests that landed on a wedged one did not return at all.
A dashboard watching means or p50s would have shown `flash-next` as the
best-performing model in the fleet on the day it was down.

## What was ruled out, and what was not

Six samples of `nvidia-smi` showed **GPU utilisation at 0% on both cards** — the
wedged workers were not computing. VRAM was **48 of 98 GB used per card**, so
the allocator was not exhausted. One wedged worker held **3 of its 4 slots
idle**, so it was not queue-saturated behind a legitimate backlog. The process's
`rchar` delta was **0 over 15 seconds**, which excludes a stalled Quobyte read.
The main thread sat in `futex_do_wait`.

That is where the evidence stops. **The mechanism inside `llama-server` was not
established.** A thread is blocked on a futex and the GPU is idle; what holds
the futex is unknown. Two leads were noted and neither was tested: the startup
log reports `n_threads = 128` on a job allocated `NumCPUs=12`, and `flash-next`
is the only model in the fleet requesting two GPUs (`gres/gpu=2`), which makes
it the only one exercising the multi-GPU path. Both are plausible and neither is
evidence. Until one of them is confirmed, rotation is mitigation, not a fix, and
the runbook should be read that way.

## SUPERSEDED (2026-09-11): do not re-enable `worker-rotate.sh`

The shell watchdog described in the next section was **disabled the same day
it was documented**, and its cron entry must not be restored. Detection moved
into the Go gateway.

**Why.** A generation probe queues behind the work it is trying to measure. On
a worker with a single slot it therefore times out on the *busiest* worker,
not the broken one. Measured: job `22999745` was marked `UNRESPONSIVE` at
`45030ms` by this watchdog while it was generating at **48.8 tok/s with 11,080
tokens in flight**. It was the healthiest worker in the fleet. Nine workers
were lost this way in one afternoon, and because rotation returns the
allocation to Slurm — where a replacement can queue for hours — every false
positive cost real capacity.

The parameters below make this worse rather than better: raising
`GEN_TIMEOUT` delays detection of genuinely dead workers without fixing the
false positives, because the probe's queueing delay is unbounded.

**What replaced it.** The gateway now separates *busy* from *stuck* by
**progress**, not response time. `llama-server`'s `/metrics` is answered off
the work queue, so it responds while the worker is mid-request:

```
job 23001707 (wedged):  prompt 8501 -> 8501,     predicted 261 -> 261,   processing 1
job 23005257 (busy):    prompt 144634 -> 155736, predicted 31835,        processing 1
```

A worker is removed only after consecutive samples showing **no token
movement**. Idle is not stuck, progress resets the count, and the first
sample concludes nothing. A busy worker cannot be caught by it, because busy
workers move tokens.

Two further corrections this class of fault taught us, both of which apply to
anything reading these counters:

- `requests_processing` is **not** a valid guard. A wedged worker reports `0`
  once the client queued behind it gives up — job 23001707 sat frozen for
  hours reporting nothing in flight.
- Read the metrics body with `io.Copy` over a `LimitReader`, never
  `io.CopyN`: `CopyN` returns `io.EOF` when the body is shorter than the
  limit, which a metrics page always is. That bug silently disabled the
  detector on every worker.

The sections below are retained as the **incident record** — what was
observed, and what was tried. Treat the operational instructions in them as
history, not as procedure.

## The watchdog: `worker-rotate.sh`

`/quobyte/metabolomicsgrp/it/llm/worker-rotate.sh` runs from cron every five
minutes. Cron for this account was consolidated onto **login1 only** on
2026-09-10 — `login2`'s crontab is deliberately empty and carries a comment
explaining why, because `ssh hive` round-robins and edits used to land on
whichever node answered. Edit it with
`ssh gw@login1.hive.hpc.ucdavis.edu crontab -e`; the entry is:

```
*/5 * * * * /bin/bash /quobyte/metabolomicsgrp/it/llm/worker-rotate.sh >> /quobyte/metabolomicsgrp/it/llm/logs/cron-worker-rotate.stderr 2>&1
```

Each sweep takes an exclusive `flock` on `state/worker-rotate.lock` and exits
immediately if another sweep holds it, so a slow sweep delays the next one
rather than overlapping it. It then walks `state/endpoints/*`. The endpoint
*filename* carries both labels — `${name%-*}` is the model and `${name##*-}` is
the Slurm job id, so `qwen3.8-27b-vision-22976281` yields model
`qwen3.8-27b-vision` and job `22976281`. Every `model` label in the telemetry
below is that filename convention, not something the worker reported about
itself. Endpoints whose job `squeue` no longer lists as ours are skipped, so the
watchdog never cancels a job it does not own.

The probe is a real generation request: `POST $base/chat/completions` with
`max_tokens: 1` and `reasoning_effort: "none"`, under `--max-time $GEN_TIMEOUT`.
Anything other than a 200 increments a per-job streak file under
`state/worker-rotate/<job>.fails`; a 200 deletes it, so only *consecutive*
failures accumulate. At `FAIL_STREAK` the worker is `scancel`led and its
endpoint file removed, which is what lets autoscale replace it. At most
`MAX_ROTATE` workers are rotated per sweep.

`GEN_TIMEOUT=45`, `FAIL_STREAK=3` and `MAX_ROTATE=2` are the defaults and all
three are environment-overridable. The cap is the load-bearing one: a systemic
fault — a bad image, a gateway outage, a Quobyte stall — presents identically to
five independently wedged workers, and without a cap the watchdog would respond
to it by cancelling the entire fleet in a single pass. Two per sweep means a
genuine fleet-wide fault drains slowly enough for a human to catch it, and a
single wedged worker is still gone within about fifteen minutes.

## Telemetry surfaces

Three files, all under `/quobyte/metabolomicsgrp/it/llm/`:

`logs/worker-rotate.log` is the human surface, one line per unresponsive worker
and one per sweep:

```
2026-09-11T11:32:12-07:00 UNRESPONSIVE qwen3.8-flash-next job=22995921 http=000 45029ms streak=1/3
2026-09-11T11:32:12-07:00 sweep done: checked=12 healthy=10 unresponsive=2 rotated=0 (gen_timeout=45s streak=3 cap=2)
```

`state/worker-health.ndjson` is the machine surface, one object per probe and
one per rotation, with `ts`, `event` (`probe` or `rotate`), `model`, `job`,
`ok`, `http`, `ms` and `streak`. It is the only place probe **latency** is
recorded.

`state/worker-health.prom` is a Prometheus textfile, rewritten atomically
(`$PROM.tmp` then `mv`) each sweep with four metrics:
`iw_worker_probe_total{model}`, `iw_worker_unresponsive{model}`,
`iw_worker_rotated_total`, and `iw_worker_rotate_sweep_timestamp_seconds`.

Two properties of these metrics will mislead anyone who reads the names instead
of the file. **Both `_total` metrics are declared `gauge` and are per-sweep, not
cumulative** — `iw_worker_rotated_total` is the count rotated in *this* sweep
and returns to 0 on the next one, so `rate()` and `increase()` over it are
meaningless. And **`iw_worker_unresponsive` emits a series only for models seen
in the current sweep**, because the emitting loop iterates the models it just
probed. A model whose endpoints have all been removed produces no series at all;
that is absence, not zero, and no threshold rule will fire on it.

### The `http: 000` rows are not valid JSON

`curl` returns the literal string `000` when it cannot complete a request, and
the NDJSON writes it unquoted as `"http":000`. JSON forbids leading zeros in a
number, so **every row recording a failed probe is rejected by a strict
parser** — confirmed against Python's `json` and V8's `JSON.parse`, both of
which raise on `{"http":000}`. `jq` happens to accept it, which is why this was
not obvious: the file validates under the tool an operator reaches for first and
fails under the parsers a log pipeline actually uses. At the time of writing 2
of 19 rows are affected, and they are precisely the rows describing the fault
the file exists to record. Quoting the value, or emitting `0`, fixes it; until
then any Loki or Infinity ingest of this file will silently drop the
interesting rows. This should be fixed before the NDJSON is wired to anything.

## Rendering it in Grafana

**Nothing is rendering this today.** There is no Prometheus and no Grafana
running on `fry`: ports 3000, 9090 and 9091 were checked and nothing is
listening. Port 9100 was not checked, so no claim is made here about whether a
`node_exporter` already exists. Everything in this section is the reader's next
step, not a description of something that works. What *is* implemented is the
watchdog and the three files above.

The two data sources also live in different places, which constrains where a
scraper can sit: `worker-health.prom` and `worker-health.ndjson` are on Quobyte
and are written from a hive login node, while `telemetry.ndjson` is on `fry` at
`/var/lib/inferweave/telemetry.ndjson`. A collector needs a path to both, or two
collectors.

### Exposing the textfile metrics

The `.prom` file is already written in the form a `node_exporter` textfile
collector expects, including the write-to-temp-then-rename that prevents a
scrape from reading a half-written file, so the shortest path is to run
`node_exporter --collector.textfile.directory=/quobyte/metabolomicsgrp/it/llm/state`
on a host with the Quobyte mount and point a scrape at its `/metrics`. The
collector globs `*.prom`, and `worker-health.prom` is the only file in `state/`
matching that pattern today, so it works unmodified — but the same directory
holds lock files, backup scripts and saved crontabs written by other things on
the cluster, and the only guarantee is that nothing else there ends in `.prom`
*right now*. Moving the emitter to a dedicated `state/textfile/` directory
removes the dependency on that staying true. Where running an exporter is not
possible,
serving the same directory over HTTP with any static file server and scraping
the file directly works too — Prometheus will parse it, and the atomic rename
makes a torn read impossible either way. Scrape at the sweep cadence; scraping
faster does not produce new information, because the values only change when a
sweep rewrites the file.

### Reading the NDJSON directly

Probe latency exists only in `worker-health.ndjson`, so the latency panels
cannot come from Prometheus without a change to the script. Grafana's Loki
datasource (with Promtail tailing the file) or the Infinity datasource (reading
the file over HTTP) both render it without any new emitter. Fix the `http: 000`
rows first: Loki's `| json` pipeline stage will drop exactly the failure rows.

### Panels worth having

*Unresponsive workers by model over time* is `iw_worker_unresponsive` graphed
directly, one series per model, no function applied — it is already a
per-sweep gauge. Read a missing series as "not probed", never as zero.

*Rotations per hour* has no honest PromQL form today, because
`iw_worker_rotated_total` resets every sweep.
`sum_over_time(iw_worker_rotated_total[1h])` is right only if the scrape
interval matches the sweep interval exactly, since it sums samples and not
events; it double-counts under a faster scrape. The reliable form is
`count_over_time({job="worker-rotate"} | json | event="rotate" [1h])` against
the NDJSON in Loki. The better fix is to make the script accumulate rotations
into a state file and emit a genuinely monotonic counter, after which
`increase()` becomes correct and the panel becomes trivial.

*Probe latency percentiles by model* must come from the NDJSON — there is no
`ms` in the `.prom` file and no histogram to quantile over. In Loki:
`quantile_over_time(0.99, {job="worker-rotate"} | json | unwrap ms [15m]) by (model)`,
with p50 and p90 alongside. This panel is the one that would have shown the
incident as it happened: the probe latencies split cleanly into sub-second
successes and 45-second timeouts, with nothing in between.

*Time since last sweep* is `time() - iw_worker_rotate_sweep_timestamp_seconds`,
and it is the most important panel on the page. It watches the watchdog. If that
number climbs past a couple of sweep intervals, the fleet has no liveness
checking at all and every other panel is showing stale values that look
healthy — which is the same failure mode as `/health`, one level up.

### The alert

One rule is enough to start:

```yaml
- alert: WorkerUnresponsive
  expr: iw_worker_unresponsive > 0
  for: 6m
  labels:
    severity: warning
  annotations:
    summary: "{{ $labels.model }}: worker answers /health but cannot generate"
```

Pick the `for:` duration against the sweep cadence, not against the scrape
interval. `for:` requires the expression to hold *continuously* for the whole
window; it does not count samples. Because the `.prom` file is only rewritten
every five minutes, a value keeps firing between sweeps whether or not the
underlying condition still holds, so any window shorter than the cadence can be
satisfied by a single sweep's value. `for: 6m` is the shortest window a single
sweep cannot satisfy alone and two consecutive sweeps do, which is the intent:
two distinct sweeps agreeing. Lengthen it only deliberately — a wedged worker
persists until the watchdog rotates it, so trading detection latency for
confidence is defensible, but record which trade was made.

Pair it with a staleness rule on
`time() - iw_worker_rotate_sweep_timestamp_seconds > 900`, which covers both a
dead cron and the model-absence gap above — a model with no series cannot fire
`WorkerUnresponsive`, but a watchdog that has stopped sweeping will fire the
staleness rule regardless of which models are present.

## Operator checklist

**Is this worker wedged, or just busy?** In under a minute:

1. Do not ask `/health`. It answers 200 in both cases; that is the entire point
   of this document.
2. Run the watchdog's own probe by hand against the endpoint in
   `state/endpoints/<model>-<job>`: a `POST` to `/chat/completions` with
   `max_tokens: 1` and `--max-time 45`. A busy worker answers, slowly. A wedged
   one returns `000` at the timeout.
3. Check the GPU on that node. A busy worker is computing; the wedged workers in
   this incident held 0% utilisation across six samples on both cards, with VRAM
   at 48 of 98 GB. Idle GPU plus an unanswered generation request is the
   signature.
4. Check slot occupancy. A worker refusing traffic because it is saturated has
   its slots full. The wedged worker had 3 of 4 idle.
5. Check `logs/worker-rotate.log` for that job's streak. If it is already at
   `2/3`, the watchdog will handle it on the next sweep and no action is needed.

**Recycling by hand when the watchdog is down.** Confirm the watchdog is
actually down first — `tail logs/worker-rotate.log` and compare the last sweep
timestamp against the five-minute cadence, since a sweep that is merely slow
will complete and rotating underneath it races the `flock`. Then reproduce what
the script does, in this order:

```bash
scancel <job>
rm /quobyte/metabolomicsgrp/it/llm/state/endpoints/<model>-<job>
```

Both steps are required. Cancelling the job without removing the endpoint file
leaves a registered endpoint pointing at nothing, and autoscale will not replace
a worker whose endpoint still appears present. Removing the endpoint without
cancelling leaves an orphaned job holding two GPUs. Also delete
`state/worker-rotate/<job>.fails` if it exists, so a recycled job id does not
inherit a stale streak.

## What this runbook does not cover

The wedge mechanism is unknown, so **rotation is mitigation and not a fix**. A
rotated worker is replaced by an identical worker running identical code on the
same hardware, and nothing established here prevents the replacement from
wedging the same way. The two untested leads — `n_threads = 128` against
`NumCPUs=12`, and `flash-next` being the only `gres/gpu=2` model — are where the
next investigation starts.

Diagnosis also required a live census of the fleet, by hand, because the
existing request telemetry could not attribute a slow request to a worker: on
all 762 rows of `telemetry.ndjson`, `worker_job` is empty and both
`prompt_tokens` and `completion_tokens` are the string `"unknown"`. The latency
table above can say *which model* was slow and cannot say *which worker*, which
is why it took a census to discover that four of five were dead rather than one
being slow. Populating `worker_job` is the single change that would most shorten
the next diagnosis, and it is outstanding.

Related: [log-status-observation](log-status-observation.md) — liveness read
from a proxy signal (log mtime) rather than from the process's own heartbeat is
the same error this document describes, one layer up the stack.
