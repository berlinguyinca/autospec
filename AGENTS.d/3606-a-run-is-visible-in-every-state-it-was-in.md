# A run is visible in every state it was in (issue #3606)

Three agent runs produced eighteen bytes and no status file:
`agent.out` held "Connection error." and nothing else — no
`status.txt`, no patch, no test log. One of them ran **3 h 03 m**
before dying, and the entire record of it was two words. And because
`status.txt` was never written, those three do not appear as failures
in any tally — they appear as *nothing*: a pipeline that reports 12
VERIFIED out of 12 status files while three runs died silently is not
reporting a success rate. The failure and the absence look identical.

The same missing signal has a mirror image that cost more: `iw-30` had
been `RUNNING` for 2 h 21 m with `agent.out` at 0 bytes, and every
observable said dead — the run was working perfectly, nine slots
generating across six workers. A supervisor with no way to tell the two
apart eventually guesses, and either guess is expensive. And the run
that produced nothing left no transcript — the session lives in
node-local scratch and is destroyed when the job ends — so a 1 h 47 m
GPU allocation left 411 bytes of key-value pairs and no way to tell
which of four causes it was: budget burned in `reasoning_content`
(`finish=length`, empty content — 36 of 68 runs), read without ever
acting, no change needed, or the harness failing underneath. Two of the
four are worth retrying and two are not, which is precisely the retry
decision the dispatcher could not make.

- **The status record is written first, not last.** It is created at
  start with `status=RUNNING`, the endpoint and the model, and updated
  on every transition, so a killed run leaves a record of where it was
  and "which worker did this run use" is answerable after the fact
  without parsing `SubmitLine`. Through the lifecycle API a record can
  only begin `RUNNING` and only settle through `finish` — there is no
  path to a record that starts at its own verdict.
- **A run with no status record is FAILED in every tally.** The
  denominator is the runs the dispatcher started, never the status
  files that exist; missing records and records still reading `RUNNING`
  are failures, and a tally whose buckets do not add up to the
  denominator is reporting a state that cannot exist (`tally`,
  `Tally::reconciles`).
- **Liveness is written periodically while work is happening** — a
  heartbeat line with a timestamp and a work counter, flushed
  unbuffered (`heartbeat_line`, `liveness`) — so a progressing run is
  distinguishable from a stopped one *without inspecting anything
  outside the run's own output*. A line whose counter does not advance
  proves the process is alive, not that work happened (`Alive`, not
  `Progressing`); silence past the window is a real signal, with the
  count (`Stalled`); and a running record with no line at all is a
  finding (`LIVENESS_UNRECORDED`) — the 2 h 21 m / 0-byte case, and the
  state a supervisor must never be put in.
- **An endpoint unreachable at start costs seconds, not a scheduled
  slot.** The dispatcher probes the endpoint before consuming a slot
  (`audit_dispatch` — a slot consumed without a probe, or past a
  failed probe, is a finding); and after the fact, a connection loss
  with no liveness line inside the fail-fast window is
  `UnreachableAtStart` — a dispatch defect — distinct from a
  preemption after progress (`classify_loss`). A loss after three hours
  with no lines is still a preemption: the endpoint answered; the
  missing lines are the separate finding.
- **A lost connection is retried against a different pool member,
  never the same one.** The retry re-reads the endpoint directory and
  picks a healthy peer (`reselect`, deterministic, fail-closed when no
  peer remains — a re-read that reports only the dead worker is a
  stale directory, not a one-member fleet); a retry to the endpoint
  the previous attempt just lost is a finding
  (`same_endpoint_retries`), while returning to a re-registered
  earlier worker is legitimate.
- **The transcript is evidence, and a failure must not destroy the
  evidence it names.** The session transcript is the one artefact that
  separates the four no-output causes, and it is copied into the
  shared output directory before the job exits whenever the run is
  about to be recorded no-output, stall-killed or non-zero
  (`transcript_policy` — the floor the issue names; copying
  unconditionally is stricter and compliant); a no-output record
  without the transcript is `DestroyedEvidence`, and with the
  transcript gone the cause is `Indeterminate`, never a guess, so the
  retry decision (`NoOutputCause::retry_worthy`) is only made on a
  separable cause. `finish_reason` and the final response's token
  counts are fields in the status file (`Terminal`, `FinishReason` —
  the parse fails closed), not a comment in a shell script.

Checkable in `autospec_core::run_lifecycle` (`StatusRecord`,
`Terminal`, `FinishReason`, `Heartbeat`, `heartbeat_line`, `liveness`,
`Liveness`, `unrecorded_liveness_finding`, `tally`, `Tally`,
`audit_dispatch`, `DispatchAudit`, `classify_loss`, `LossClass`,
`reselect`, `same_endpoint_retries`, `transcript_policy`,
`transcript_verdict`, `classify_no_output`, `NoOutputCause` — pure
in-memory, no subprocess, so the shell runner can adopt them as the
single source of truth). Tests:
`crates/autospec-core/tests/run_lifecycle.rs`, including the regression
that reconstructs the incident end-to-end: three dispatched runs with
no status record (the 18-byte case, and the 12/15 line the fixed tally
prints instead of the incident's 12/12), the 3 h 03 m preemption that
leaves a record of where it was, the dispatch against a worker that no
longer existed, the 2 h 21 m unrecorded-liveness state, and the 1 h 47
m no-output run with the transcript destroyed.
