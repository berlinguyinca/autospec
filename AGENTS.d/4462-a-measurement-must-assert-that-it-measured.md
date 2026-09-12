# A measurement must assert that it measured (issue #4462)

Verification code is held to a lower standard than the code it verifies,
because it is written to be thrown away. But it is the thing that decides
what is true. A bug in the product produces a wrong result; a bug in the
verification produces a wrong *belief*, which survives longer and
propagates into issues, reports, and decisions made downstream.

Four measurement bugs in one session, all in throwaway verification code,
none in the product:

1. A model probe timed out and `curl -o /tmp/g.json` wrote nothing, so the
   parser read the *previous* model's response and reported `READY` for a
   model that had actually failed with `http=000`.
2. Ten concurrent requests; the per-request `read` of the status files
   failed, so the tally printed `ok=0 failed=10` while the same output's
   body check printed `non-empty content: 10/10`. The service was fine; the
   counting was broken, and the two fields contradicted each other in the
   same output.
3. A background test run was terminated when its tool call timed out,
   leaving `suites=0 FAILED=0` — a killed gate read as zero failures, for
   the third time. The only reason it was caught was a completion marker
   added earlier in the same session, and it had to be caught three times.
4. A fleet scan reported `WEDGED` from a single flat sample; a second,
   longer sample showed decode advancing by 890 tokens and a direct
   generation returning in 2 seconds. The worker was healthy; the scan had
   sampled a gap between requests. **The correct rule — "persistence
   across N consecutive samples" — had been written into the acceptance
   criteria of the issue filed about exactly this, and then omitted from
   the implementation**, because writing a rule into an issue and applying
   it to one's own tooling are different acts.

## The bias

In all four cases the broken measurement produced the answer that invites
no further checking: a working model, a clean gate, a stopped
investigation. A measurement bug that produced an alarming reading would
have been investigated immediately. **So a clean result from new
verification code is the case that warrants a second look, not the case
that ends it.**

## The rule

**A measurement must assert that it measured.** Each assert has a concrete
shape:

- **A parse of an output file asserts the file was written *by this
  invocation*** — remove it first, or check its mtime — never read a path
  that a previous iteration may have populated.
- **A tally asserts its own arity:** `ok + failed == attempted`. Any tally
  that can print `0 of 10` while another field in the same output says
  `10 of 10` succeeded is not a measurement.
- **A run emits a completion marker, and its absence yields *unmeasured*,
  never pass or fail.** Counters from a run that may have been killed are
  a lower bound on activity, not a verdict.
- **A verdict from a sampled time series requires persistence across
  consecutive samples before it is actionable; one window is never
  sufficient.** A single flat sample can be a gap between requests; a
  single active sample can be a one-off.

Checkable in `autospec_core::measurement_assertion`
(`capture_verdict`/`CaptureVerdict`, `Tally`/`TallyArityError`/
`tally_contradiction`, `marked_run`, `PersistenceGate`). Tests:
`crates/autospec-core/tests/measurement_assertion.rs`, including each
incident end-to-end: the previous model's response is `Stale`, the
`ok=0 failed=10` beside `10/10` pair is a contradiction, the killed run
with `FAILED=0` is `NotMeasured` rather than `Pass`, and one flat sample
never gates a `WEDGED` verdict.
