# A metric that only updates on completion cannot detect progress during a long operation (issue #4407)

A stuck-worker detector sampled `llamacpp:prompt_tokens_total` and concluded
"no progress" when it did not move. But that counter is incremented when a
request **completes**, not as tokens are processed. A worker in the middle of
a 40-minute prefill therefore shows a frozen counter while doing exactly the
work it is supposed to do. The detector classified two healthy workers as
STUCK; they were logging progress lines the whole time
(`prompt processing, n_tokens = 4096, progress = 0.10`).

This is the third distinct false-positive mode found in the same detector:

1. response-time probes time out on **busy** workers (they queue behind real
   work);
2. `requests_processing` reads 0 on a **wedged** worker once its client gives
   up;
3. **completion-keyed** counters freeze during a long in-flight operation.

The three modes look identical in the detector's output — a signal that says
"no progress" — and each is a different cause. A detector that reads a frozen
counter as "stopped" is reading the *normal in-flight state* of a
completion-keyed counter as evidence of a stop, and it will do so exactly
when the worker is busiest.

- **A progress signal must be incremented by the work, not by the completion
  of the work.** Before using a counter as a liveness signal, establish its
  update granularity — per unit of work, or per completed unit
  (`UpdateGranularity`, `Metric`)? Only the per-unit signal advances while the
  operation is in flight, so only it can distinguish "slow" from "stopped"
  (`liveness_capable`), and that distinction is the entire purpose of the
  check.
- **A total is frozen by definition while its operation is in flight.**
  `UpdateGranularity::mid_operation` states it: a per-unit signal advances, a
  total is frozen. A detector built on totals therefore must additionally
  require that the worker is **not** reporting in-flight progress before
  concluding anything. `classify_stuck` makes the asymmetry structural: a
  total can only ever yield `StuckVerdict::InFlight` (the worker is reporting
  progress — the incident) or `StuckVerdict::Inconclusive` (no report — a
  total still cannot call it), never `StuckVerdict::Stopped`.
  `StuckVerdict::is_stuck` returns true only for a frozen per-unit signal.
- **Where no per-unit counter exists, the correct signal is the one the
  process emits as it works** — a progress log line, a partial-result
  callback, a per-chunk metric — not a total. `InFlight` is that signal, and
  it is the fallback `classify_stuck` reads for a total. In this detector the
  slot progress lines and `n_decode_total` update during the operation;
  `prompt_tokens_total` and `tokens_predicted_total` do not.
- **A spec that says "detect a stalled X by watching metric M" must state M's
  update granularity and what M does while X is mid-operation.** Otherwise the
  detector is specified against a metric whose semantics nobody checked —
  which has now happened three times on one component. `LivenessSpec` is the
  check: a spec that omits the mid-operation behavior, or states it wrong
  ("advances" for a total), is a finding (`LivenessSpec::finding`).

Checkable in `autospec_core::progress_signal` (`UpdateGranularity`,
`MidOperation`, `Metric`, `liveness_capable`, `InFlight`, `StuckVerdict`,
`classify_stuck`, `LivenessSpec`). Tests:
`crates/autospec-core/tests/progress_signal.rs`, including the regression
that reconstructs the incident end to end: the frozen completion-keyed total
with the worker logging `progress = 0.10` is `InFlight`, not stuck, while the
per-unit `n_decode_total` is the only signal that can distinguish slow from
stopped.
