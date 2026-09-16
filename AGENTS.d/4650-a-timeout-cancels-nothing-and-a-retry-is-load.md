# A timed-out request cancels nothing, and a retry loop is load (issue #4650)

A model appeared unresponsive. Each probe timed out client-side, so the author
probed again — about ten times across several minutes, at 90–180s each. Every
one of those requests was still running on the server after the client gave up.
The server had **one** generation slot, so the retries queued behind each other,
and the queue being measured was the queue being created. The model looked
progressively worse the harder it was checked; a later single probe, after
waiting for the backlog to drain, still timed out because the backlog had not
drained.

The underlying worker was genuinely misconfigured — there was a real problem.
But the evidence used to reason about it was substantially the author's own
load, and the two could not be told apart while it was being generated.

## The invariants

1. **A client-side timeout cancels nothing.** The request you gave up on is
   still running and still holds a slot. Never retry a slow request against a
   capacity-limited backend without cancelling the first or accounting for it as
   capacity still being consumed.
2. **Probe with the cheapest request that answers the question.** `/health` and
   `/slots` are free and would have shown `1 slot, busy` immediately. The answer
   was one cheap call away throughout, and generation requests were used instead.
3. **Never issue N probes where one would do.** A retry loop against an unknown
   backend is a load test, and a load test of a saturated service returns the
   saturation you added.
4. **Read capacity before interpreting latency.** Latency is only meaningful
   relative to concurrency; against a single slot, the second concurrent request
   is indistinguishable from a hang.

## The general shape

This is the diagnostic equivalent of the observer disturbing the system, and it
appears wherever an agent measures something it can also load: retrying a slow
database query, re-running a saturated build, re-requesting a rate-limited API.
The failure mode is characteristic — the evidence gets worse as you gather more
of it, which reads as a worsening incident and invites more probing.

A useful rule: if a system looks worse each time you check it, suspect the
checking before you suspect the system.

## Where it is enforced

- `implementer-contract.md` gained the `PROBE_CONTRACT` corrective directive
  (directive-only, the #4591 `PERF_CONTRACT` / #4555 `REAPER_CONTRACT` pattern):
  a timeout cancels nothing; probe with the cheapest request that answers the
  question; never issue N probes where one would do; read capacity before
  interpreting latency.
- `AGENTS.md` (Service-address resolution and pool health) records the same
  discipline next to the existing service-health guidance: a timed-out request
  still holds a slot, health/capacity endpoints are the first probe for "is
  this thing serving", and repeated expensive probes against one endpoint are
  themselves load.
