# 4650 — a timed-out request is still running, and a retry loop is load

A model probe loop timed out client-side, retried ~ten times at 90–180s each,
and every one of those requests was still running on the server — which had a
single generation slot. The retries queued behind each other, so the queue being
measured was the queue being created, and the saturated model looked worse the
harder it was checked.

The prompts now pin the probe discipline before the evidence is trusted:

- `implementer-contract.md` gained a `PROBE_CONTRACT` corrective directive
  (directive-only, the `PERF_CONTRACT`/`REAPER_CONTRACT` pattern): a client-side
  timeout cancels nothing — the in-flight request still holds a slot, so cancel
  or account for it before retrying; probe with the cheapest request that
  answers the question (`/health`, `/slots` come before any generation
  request); never issue N probes where one would do; read capacity before
  interpreting latency; suspect the checking before the system when a system
  looks worse each time it is checked.
- `AGENTS.md` (Service-address resolution and pool health) records the same
  discipline next to the existing service-health guidance.
- `AGENTS.d/4650-a-timeout-cancels-nothing-and-a-retry-is-load.md` records the
  incident and the four invariants.
