### Added

- `autospec-core::spot_measurement` — primitives for the invariant that
  a measurement the system takes continuously beats one you take by
  hand (issue #4444: a hand-measured 4–6 tok/s prefill floor and one
  0% GPU spot check produced a filed architectural claim that the
  instrumented admission probe refuted within minutes — 222 tok/s, 37x
  above the floor, not CPU speed for a 107 GB model). `adjudicate`
  compares a hand-taken reading against the system's own record for
  the same subject: agreement within `DISAGREEMENT_TOLERANCE` holds,
  disagreement is `InstrumentedWins` with the ratio between the
  readings (infinite when one side read zero — the 0% gauge in a gap
  between compute phases), and a subject the instrument has not
  logged is `NoInstrumentation`, never a finding. `permitted_status`
  gives a spot-check provenance `Provisional` no matter how many times
  the hand took it, and a continuous instrument `Finding`; `judge_claim`
  renders the incident's filed claim as `UnwarrantedGeneralisation`
  (system-scoped, spot-check provenance, recorded as a finding), an
  unrecorded provenance as `Unmeasured`, and a count of one as
  `SingleSample` — flagged revisitable, because recording *how* a
  claim was measured and *how many times* is what makes it possible to
  notice later that it was one sample. `Provenance::new` refuses a
  count of zero, and `audit_sequence` flags a conclusion drawn before
  any instrumentation existed (`ConcludedBeforeInstrumenting`) —
  instrument before concluding, not only before fixing.
