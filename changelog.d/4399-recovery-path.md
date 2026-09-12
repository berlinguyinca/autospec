### Added

- `autospec-core::recovery_path` — primitives for the invariant that a
  recovery path must not depend on an artifact produced by the step it
  recovers (issue #4399: a worker probed its model server once, 10 seconds
  after start, and treated the failure as permanent; `/props` answers 503
  until the weights are resident, `qwen3.8-flash-next` is a 105 GiB mmap
  from a network filesystem that had not finished after 1h44m, so the
  endpoint file was never written and four workers held 8 GPUs, fully
  loaded and invisible to the gateway). The comment said "regsweep will
  retry within 5 minutes" — false on this path, because regsweep
  reconciles *from* the endpoint files: the retry loop and the failing step
  share a dependency, so the retry can never fire for the case that
  actually needs it. `recovery_verdict` classifies a recovery as
  `Decorative` when it reads an artifact the step it recovers produces,
  `Covers` when every input survives the failure, and fail-closed
  `Unverifiable` when the recovered step is not in the model.
  `readiness_verdict` enforces that waiting for a slow dependency is a
  wait, not a probe: one check of a resource with a startup phase is
  `OneShot`, a poll to a deadline inside the measured worst case is
  `GuaranteedFailure` — 10 seconds for something that takes an hour is not
  a conservative choice, it is a guaranteed failure. `claim_verdict` makes
  a comment asserting that something else will retry a claim that must be
  verified against the recovery it names (`Verified` / `False` /
  `NamesNothing` / `Unverifiable`), and `RegistrationSpec::findings`
  requires a "register on startup" spec to state what happens when the
  thing is not ready yet, the measured worst case of readiness (a bound
  with no source is a guess), and who retries — naming the input that
  retry depends on (#4399, 2026-09-11).
