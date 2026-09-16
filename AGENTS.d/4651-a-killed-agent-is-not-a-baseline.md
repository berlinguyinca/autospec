# A killed agent is not a baseline

Issue #4651: `iw-87` ran 3602 s and was terminated mid-edit — 12 files
half-applied, `agent_rc=143`, `fmt_rc=1`, `test_rc=101`, and an empty report.
`143` is `128 + 15`, SIGTERM. The runner bounds itself with
`timeout -k 60 25200`, whose expiry exits `124`, so the runner had not ended
the run: something else did. The record said so precisely, and the reader
discarded the field that said it.

## The defect

The report reader parsed `status`, `build_rc`, `test_rc`, `fmt_rc` and
`fmt_files`, and ignored `agent_rc`. A killed agent therefore entered triage
carrying the stage fields of a half-finished edit and no fact about its own
death:

- `fmt_rc=1` with `build_rc=0` triaged as a repairable formatting failure, so
  the pass **formatted the 2000-line half-applied patch and offered it to the
  conversion queue**;
- a run that died before writing a label fell through the exhaustive match to
  the green arm, because a status the vocabulary does not declare resolves to
  `None` and `None` means "confirm against current main";
- two runs that produced nothing at all recorded `NO-OUTPUT`, which says
  nothing about *why* there was nothing, and the third recorded
  `UNKNOWN-NO-BASELINE` — a claim about the baseline, from a run that never
  reached one.

The cost was three Phase-3 tasks, an hour of GPU time each, and no diagnosis
afterwards.

## The fix

`SIGNALLED` is an emitted status in the run-status vocabulary, and the
termination is read from the exit code: `124` is the runner's own bounding
`timeout` (a known sender), `128 + N` is a signal from somewhere that has not
identified itself. Triage answers with a terminal `Signalled` decision whose
line carries the attribution — which signal, and that the runner's own
timeout did not fire — and `dispatch_guard::classify_report` treats the
artifact as a failed run so the slot frees and the evidence is preserved
rather than held against a conversion that will never come.

## The invariant

1. **A termination must be attributable, or say that it is not.** `124` names
   its sender. `128 + N` names a signal and no sender, and that absence is a
   finding, not a default. An exit code that is dropped by the reader cannot
   attribute anything.

2. **The exit code outranks the label.** A runner that dies before writing its
   verdict leaves only the code, and a code cannot be wrong about how the
   process died (#4206 states the same rule for `test_rc`: a record saying
   `VERIFIED` beside `test_rc=101` is triaged on the code).

3. **A signal outranks the stage fields.** A killed run reached no stage, so
   an `fmt_rc` beside it is debris from a half-finished edit. Repairing it is
   not triage; it is formatting someone else's interrupted work and shipping
   it.

4. **"Nothing was measured" is refused, not held.** A hold asserts a
   deterministic property of the submission. A killed agent established none,
   so the patch is neither a verdict nor a re-dispatch — the retry walks into
   the same supervisor.

5. **Two supervisors of one process must agree, and the code must say which
   is right.** `agent_watchdog.rs` (#4258) invariant 4 already forbids the
   inference that killed these runs: *"a working process can also show an
   empty redirect target, so 'the log is empty' is never a hang signal."* A
   45-minute watchdog that decides on the mtime of the two files that go quiet
   during any long model generation violates it, while the sibling component
   uses 450 minutes for the same reason.

## The general rule

Before you act on a record, ask what would have been in it if the producer had
died halfway. If the answer is "the same fields, minus one," then that one
field is the verdict and everything else is debris — read it first, and refuse
rather than infer. Silence about a cause is a result to report, not a gap to
fill with the nearest plausible explanation.
