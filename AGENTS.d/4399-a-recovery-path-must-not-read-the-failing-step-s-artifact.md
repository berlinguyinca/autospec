# A recovery path must not depend on an artifact produced by the step it recovers (issue #4399)

A worker probed a dependency **once**, 10 seconds after starting it, and
treated the failure as permanent.

`worker.sh` reads the served context from the model server:

```sh
CTX_PER_SLOT=$(curl -s --max-time 10 "$ENDPOINT/props" ... )
```

`/props` answers `503 {"error":"Loading model"}` until the weights are
resident. `qwen3.8-flash-next` is 105 GiB of mmap from a network filesystem
and had not finished after **1h44m**. So the probe always failed, the
endpoint file was never written, and four workers sat holding **8 GPUs**,
fully loaded and completely invisible to the gateway.

The code carried this comment:

```text
REGISTER FAILED http=$code -- regsweep will retry within 5 minutes
```

That reassurance is **false on this path**. `regsweep` reconciles *from* the
endpoint files — and the endpoint file is only written once the probe
succeeds. The recovery mechanism consumes the artifact that the failing step
was supposed to produce.

So the failure is not "a retry was missing". It is that **the retry loop and
the failing step share a dependency, so the retry can never fire for the case
that actually needs it.** The reconciler covers the easy failure (gateway
briefly down) and is structurally incapable of covering the hard one (model
still loading). A missing retry is a bug a test catches; a retry that reads
its own precondition is a design defect that every test passes, because in
every test the dependency is already up.

- **Waiting for a slow dependency is a wait, not a probe.** Any check
  against a resource with a startup phase must poll to a deadline, and the
  deadline must be scaled to the resource — 10 seconds for something that
  takes an hour is not a conservative choice, it is a guaranteed failure.
  The measured worst case is the floor of the deadline, not a detail.
- **A recovery path must not depend on an artifact produced by the step it
  recovers.** Write down, for every reconciler, what it reads and which
  failures can prevent that input from existing. If the answer includes the
  failure being recovered, the recovery is decorative: the reconciler exists
  in the code, the name is in the log line, and the retry can never fire for
  the one case that needs it.
- **A comment asserting that something else will retry is a claim that must
  be verified.** It shaped the behaviour of everyone reading the code — it
  is the reason nobody looked — and it was wrong. Verifying it is cheap:
  name the recovery, name its inputs, check that the failure being recovered
  cannot prevent those inputs from existing.
- **A spec that says "register on startup" must state the readiness
  policy**: what happens when the thing being registered is not ready yet,
  how long readiness may take (with the measured worst case, not a guess),
  and who retries — naming the input that retry depends on. A spec that
  omits all three produces exactly the code above, with exactly the comment
  above, because the implementer had no number to scale the deadline to and
  no input to point the retry at.

Checkable in `autospec_core::recovery_path` (`readiness_verdict` — one check
of a resource with a startup phase is `OneShot`, a poll to a deadline inside
the measured worst case is `GuaranteedFailure`; `recovery_verdict` — a
recovery that reads an artifact its recovered step produces is
`Decorative`, fail-closed `Unverifiable` when the step is not in the model;
`claim_verdict` — a retry comment verified against the recovery it names;
`RegistrationSpec::findings` — the three things a "register on startup" spec
must state). Tests:
`crates/autospec-core/tests/recovery_path.rs`, including the incident
end-to-end: the one-shot 10-second probe against a 1h44m measured load is a
probe, not a wait; `regsweep` reading the endpoint file is decorative and the
comment is a false claim; the same reconciler pointed at the scheduler's job
record covers the hard failure and the same claim verifies.
