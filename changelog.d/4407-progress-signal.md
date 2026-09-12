### Added

- `autospec-core::progress_signal` — primitives for the invariant that a
  progress signal must be incremented by the work, not by the completion of
  the work (issue #4407: a stuck-worker detector sampled
  `llamacpp:prompt_tokens_total` and read a frozen counter as "no progress",
  but that counter is incremented when a request completes, not as tokens are
  processed — a worker mid-prefill on a 40-minute request shows a frozen
  counter while doing exactly the work it is supposed to do, and two healthy
  workers were classified STUCK while logging `progress = 0.10` the whole
  time). The update granularity of a metric is established up front
  (`UpdateGranularity::PerUnit` vs `PerCompletion`); only a per-unit signal
  advances while its operation is in flight and so can distinguish "slow"
  from "stopped" (`liveness_capable`, `UpdateGranularity::mid_operation`).
  `classify_stuck` reads a metric, its movement between two samples, and the
  process's own in-flight report (`InFlight`) into a `StuckVerdict`, and the
  asymmetry is structural: a total can only ever yield `InFlight` (the worker
  is reporting progress — the incident) or `Inconclusive` (no report — a
  total still cannot call it), never `Stopped`, so `is_stuck` is true only
  for a frozen per-unit signal. A detector built on totals must additionally
  require that the worker is not reporting in-flight progress before
  concluding anything. `LivenessSpec` is the "for specs" rule: a spec that
  says "detect a stalled X by watching metric M" must state M's update
  granularity and what M does while X is mid-operation, or it is a finding
  (#4407, 2026-09-11).
