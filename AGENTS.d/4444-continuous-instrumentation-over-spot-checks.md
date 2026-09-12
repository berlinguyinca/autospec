# Prefer continuous instrumentation over hand measurement: a 0% GPU spot check produced an architectural claim that was wrong by 37x (issue #4444)

Across a session I measured `qwen3.8-flash-next` prefilling at
**4–6 tok/s** on several workers, and observed **0% GPU utilisation
with idle power draw** during one such prefill. I concluded that
llama.cpp had no GPU path for the architecture and was executing the
model on CPU — an architectural claim, filed and argued in detail.

Then I instrumented the admission probe to log its throughput decision
on every path. The first eight measurements, on uniquely-generated
uncacheable prompts:

```text
qwen3.8-27b         799 … 1797 tok/s
qwen3.8-27b-vision  798 tok/s
qwen3.8-flash-next  222 tok/s      <- 37x above the floor
```

222 tok/s is not CPU speed for a 107 GB model. The conclusion was
wrong, or at best described specific degraded workers rather than the
model.

The 0% GPU reading felt decisive — it is exactly what CPU execution
looks like — and I stopped looking once I had it. But a spot sample of
a utilisation gauge during a long operation can land in a gap between
compute phases, and it was taken once, on one worker, at one moment.

Every individual observation was true. The generalisation from
"these workers were slow on these prompts" to "the runtime cannot use
the GPU for this architecture" was the error, and it upgraded a local
symptom into a claim about an upstream project. A hand-taken
measurement samples whatever the system happened to be doing;
instrumentation samples what it does. The conclusion was built on the
former and was wrong within minutes of the latter existing.

- **Prefer a measurement the system takes continuously over one you
  take by hand.** Where they disagree, the instrument wins: it samples
  the population, you sampled an occasion.
- **A conclusion drawn from spot checks is provisional until
  something measures it repeatedly.** Write it down with that status,
  rather than as a finding — especially before escalating it to a
  claim about someone else's software.
- **Instrument before concluding, not only before fixing.** The same
  rule that said "instrument the decision before tuning the threshold"
  applies to diagnosis: the instrumentation here cost one small change
  and refuted a conclusion that had already consumed hours and
  produced a filed architectural claim.
- **Record how a claim was measured, and how many times.** An agent
  cannot easily revisit a measurement it took yesterday, and has every
  incentive to build on its own recorded conclusions. That makes
  premature generalisation unusually expensive here: the wrong
  conclusion becomes an input to later reasoning and to other issues.
  The recorded count is what makes it possible to notice later that a
  claim was one sample.

Checkable in `autospec_core::spot_measurement` (`adjudicate` — a
hand-taken reading against the system's own record for the same
subject: agreement within `DISAGREEMENT_TOLERANCE` holds, disagreement
is `InstrumentedWins` with the ratio, and no record for the subject is
`NoInstrumentation`, never a finding; `permitted_status` — a
spot-check provenance permits only `Provisional`, no matter how many
times the hand took it, while a continuous instrument permits
`Finding`; `judge_claim` — a system-scoped claim on spot-check
provenance recorded as a finding is `UnwarrantedGeneralisation`, an
unrecorded provenance is `Unmeasured`, and a count of one is
`SingleSample`, flagged revisitable; `Provenance::new` refuses a count
of zero; `audit_sequence` — a `Concluded` step before any
`Instrumented` step is `ConcludedBeforeInstrumenting`). Tests:
`crates/autospec-core/tests/spot_measurement.rs`, including the
incident end-to-end: the filed architectural claim is
`UnwarrantedGeneralisation`, the instrumented record wins by 37x, and
the same statement re-recorded on the instrument's provenance stands
as a finding.
