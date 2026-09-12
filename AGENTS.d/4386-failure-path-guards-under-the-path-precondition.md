# A guard on a failure path must be reasoned about under that path's precondition (issue #4386)

A guard was written to be conservative, and because it was only ever
reached on a failure path, it was conservative about the wrong thing.

A stuck-worker detector concluded "stalled" from frozen token counters.
It also required the worker to report a request in flight:

```go
if !cur.busy() || cur.advancedOver(prev) { /* not stuck */ }
```

The `busy()` guard reads as obvious prudence: do not call an idle worker
broken. But `observe()` is reached ONLY after a liveness probe has already
FAILED, and a healthy idle worker answers that probe immediately — so it
never gets there at all. Within that branch "idle" does not mean healthy,
it means *refusing to start work*, which is a worse fault than stalling
mid-request, not a lesser one.

Measured: a worker sat at `prompt=8501 predicted=261`, unchanged across
hours, failing every probe, reporting `requests_processing=0` because the
client queued behind it had given up. The guard excused it indefinitely.
It was one of nine workers lost that afternoon.

- **A predicate evaluated only on a failure path must be reasoned about
  under the precondition of that path, not in general.** States that are
  benign in the general population — idle, empty, zero — are frequently
  diagnostic once you know how you arrived.
- **Write the precondition down at the top of such a function.**
  "Reached only after X has failed" is load-bearing and invisible at the
  call site.
- **For specs:** when a spec says "detect condition C", it must also say
  what is already known to be true wherever the detection runs. A detector
  specified without its precondition gets written as if it ran everywhere,
  and its guards get calibrated against the wrong base rate.
- **For reviewers:** for any guard, ask "on the path that actually
  reaches this line, is the excused state still innocent?"
