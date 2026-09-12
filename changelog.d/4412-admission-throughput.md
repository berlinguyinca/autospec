### Added

- `autospec_core::aar::admission_throughput` — the admission throughput
  probe's two invariants, for the lesson that a check whose non-firing is
  indistinguishable from its absence cannot be debugged, only guessed at
  (#4412, 2026-09-12):
  1. The probe prompt is unique per probe — `probe_prompt` prefixes the
     fixed body with a per-probe nonce (failing closed on a blank nonce or
     body), and `prompt_shape` classifies the prompts actually sent as
     `UniquePerProbe` or `Reused`, because a constant prompt can be served
     from `llama-server`'s prompt cache and its "prefill rate" measures a
     cache lookup, not computation.
  2. The admission decision renders a line on every path —
     `decide_admission` maps a `Measured` rate to `Verified` (at/above the
     floor) or `Refused` (below it) and a `NotMeasured` probe to the named
     `Bypassed` branch, and `AdmissionDecision::line` renders
     measured/not-measured, the rate, the floor, and the branch on all
     three, so "it worked" and "it never ran" no longer look identical in
     the gateway log.
